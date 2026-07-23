use std::{env, sync::Arc, time::Duration};

use anyhow::Result;
use bookmeter::{BookMeterBook, BookMeterClient};
use tracing::{error, info};

mod bookmeter;
mod isbn;
mod kindle;
mod metrics;
pub mod model;
pub mod used_book;
pub mod used_book_offer;
use futures::{Stream, TryStreamExt};
use kindle::Kindle;
use model::Entity as Book;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, ExprTrait, IntoActiveModel,
    QueryFilter, QueryOrder, QuerySelect, Set,
};
use tokio::time::sleep;
use used_book::UsedBookSite;
use used_book_offer::Entity as UsedBookOffer;

pub struct BookMeterDiscounts {
    pub user_id: String,
    pub db: DatabaseConnection,
    pub get_amazon_page_interval: u64,
    metrics: Arc<metrics::MetricsCollector>,
}

impl BookMeterDiscounts {
    #[must_use]
    pub fn new(user_id: &str, db: DatabaseConnection, get_amazon_page_interval: u64) -> Self {
        let metrics = metrics::MetricsCollector::new();
        Self {
            user_id: user_id.to_string(),
            db,
            get_amazon_page_interval,
            metrics,
        }
    }

    /// # Errors
    ///
    /// Returns an error if updating or fetching discounts fails.
    pub async fn update_and_get_discounts(
        &self,
    ) -> Result<impl Stream<Item = Result<model::Model>> + '_> {
        self.update_discounts().await?;
        self.get_discounts(Some(10)).await
    }

    /// # Errors
    ///
    /// Returns an error if any database or network operation fails.
    ///
    /// # Panics
    ///
    /// This function does not panic.
    #[expect(
        clippy::too_many_lines,
        reason = "sequential update steps (fetch, delete, kindle id, price, binding name, used book offers) read clearest inline"
    )]
    pub async fn update_discounts(&self) -> Result<()> {
        // 読書メーターから本情報の取得
        let max_page = env::var("MAX_PAGE")
            .unwrap_or_else(|_| "1".to_string())
            .parse()?;
        let bookmeter_client = BookMeterClient::new(self.user_id.parse()?);
        let wishlist_ids = bookmeter_client.fetch_wishlist_ids(max_page).await?;
        let new_books = bookmeter_client
            .fetch_new_books(&wishlist_ids, &self.db)
            .await?;
        for bookmeter_book in new_books {
            let book = model::ActiveModel::from(bookmeter_book);
            if let Err(e) = Book::insert(book).exec(&self.db).await {
                error!("{:?}", e);
            }
        }

        // 読書メーターから削除済みの本の削除
        // ウィッシュリスト取得が空の場合はスクレイピング失敗の可能性があるため、保険として削除をスキップする
        if !wishlist_ids.is_empty() {
            let to_delete = Book::find()
                .filter(model::Column::BookmeterId.is_not_in(wishlist_ids.iter().copied()))
                .all(&self.db)
                .await?;
            if !to_delete.is_empty() {
                for book in &to_delete {
                    info!("delete book: {}", book.title);
                    self.metrics.record_deleted_book();
                }
                Book::delete_many()
                    .filter(model::Column::BookmeterId.is_not_in(wishlist_ids.iter().copied()))
                    .exec(&self.db)
                    .await?;
            }
        }

        // kindle idとKindle Unlimited判定の取得
        let mut stream = Book::find()
            .filter(
                model::Column::ActiveAt
                    .is_null()
                    .or(model::Column::ActiveAt.lte(chrono::Utc::now().naive_utc())),
            )
            .stream(&self.db)
            .await?;
        while let Some(item) = stream.try_next().await? {
            let book: model::Model = item;
            let mut active_book = book.clone().into_active_model();
            if book
                .active_at
                .is_some_and(|active_at| active_at > chrono::Utc::now().naive_utc())
            {
                info!("skip getting kindle edition for {}", book.title,);
                continue;
            }
            sleep(Duration::from_secs(self.get_amazon_page_interval)).await;
            let kindle_edition =
                match Kindle::convert_amazon_url_to_kindle_id(&book.amazon_url).await {
                    Ok(kindle_edition) => kindle_edition,
                    Err(e) => {
                        info!(
                            "error while getting kindle edition from {}: {:?}",
                            book.amazon_url, e
                        );
                        if e.to_string().contains("Kindle button not found") {
                            active_book.active_at = Set(Some(
                                chrono::Utc::now().naive_utc() + chrono::Duration::days(30),
                            ));
                            active_book.updated_at = Set(chrono::Utc::now().naive_utc());
                            active_book.update(&self.db).await?;
                        }
                        continue;
                    }
                };
            active_book.kindle_id = Set(Some(kindle_edition.kindle_id));
            active_book.is_kindle_unlimited = Set(kindle_edition.is_kindle_unlimited);
            active_book.updated_at = Set(chrono::Utc::now().naive_utc());
            active_book.update(&self.db).await?;
            self.metrics.record_kindle_id_fetched();
        }

        // kindle id取得済みの本の価格を取得
        let mut stream = Book::find()
            .filter(model::Column::KindleId.is_not_null())
            .order_by_asc(model::Column::UpdatedAt)
            .stream(&self.db)
            .await?;
        while let Some(item) = stream.try_next().await? {
            sleep(Duration::from_secs(self.get_amazon_page_interval)).await;
            let mut book: model::ActiveModel = item.into();
            let kindle_id = book
                .kindle_id
                .clone()
                .into_value()
                .ok_or_else(|| anyhow::anyhow!("kindle_id is None"))?
                .to_string();
            let kindle = match Kindle::from_id(&kindle_id).await {
                Ok(kindle) => kindle,
                Err(e) => {
                    info!("error while getting kindle price from {kindle_id}: {e:?}",);
                    continue;
                }
            };
            book.basis_price = Set(Some(i32::try_from(kindle.basis_price)?));
            book.price = Set(Some(i32::try_from(kindle.price)?));
            book.discount_rate = Set(Some(kindle.discount_rate));
            book.updated_at = Set(chrono::Utc::now().naive_utc());
            book.update(&self.db).await?;
            self.metrics.record_price_fetched();
        }

        // 書籍の形式 (binding_name) が未取得の本の形式を取得
        let mut stream = Book::find()
            .filter(model::Column::BindingName.is_null())
            .stream(&self.db)
            .await?;
        while let Some(item) = stream.try_next().await? {
            let book: model::Model = item;
            sleep(Duration::from_secs(self.get_amazon_page_interval)).await;
            let Ok(bookmeter_id) = u32::try_from(book.bookmeter_id) else {
                continue;
            };
            let binding_name = match BookMeterBook::fetch_binding_name(bookmeter_id).await {
                Ok(binding_name) => binding_name,
                Err(e) => {
                    info!(
                        "error while getting binding name for {}: {:?}",
                        book.title, e
                    );
                    continue;
                }
            };
            info!("binding name of {}: {:?}", book.title, binding_name);
            // 形式が取得できなかった場合は NULL のままにし、次回実行時に再取得する
            let Some(binding_name) = binding_name else {
                continue;
            };
            let mut active_book = book.into_active_model();
            active_book.binding_name = Set(Some(binding_name));
            active_book.updated_at = Set(chrono::Utc::now().naive_utc());
            active_book.update(&self.db).await?;
        }

        // 漫画・ライトノベル以外の本の中古本オファーを取得
        let mut stream = Book::find()
            .filter(model::Column::BindingName.is_not_null())
            .filter(model::Column::BindingName.is_not_in(["コミック", "ライトノベル"]))
            .stream(&self.db)
            .await?;
        while let Some(item) = stream.try_next().await? {
            let book: model::Model = item;
            let isbn13 = match Kindle::convert_amazon_url_to_id(&book.amazon_url)
                .and_then(|asin| isbn::isbn10_to_isbn13(&asin))
            {
                Ok(isbn13) => isbn13,
                Err(e) => {
                    info!(
                        "skip used book offers for {} (invalid ISBN from {}): {:?}",
                        book.title, book.amazon_url, e
                    );
                    continue;
                }
            };
            for site in UsedBookSite::ALL {
                sleep(Duration::from_secs(self.get_amazon_page_interval)).await;
                if let Err(e) = self.update_used_book_offer(&book, site, &isbn13).await {
                    info!(
                        "error while updating used book offer of {} on {}: {:?}",
                        book.title,
                        site.as_str(),
                        e
                    );
                }
            }
        }

        Ok(())
    }

    /// 1冊・1サイト分の中古本オファーを取得してDBに保存する
    ///
    /// # Errors
    ///
    /// Returns an error if the network or database operation fails.
    pub async fn update_used_book_offer(
        &self,
        book: &model::Model,
        site: UsedBookSite,
        isbn13: &str,
    ) -> Result<()> {
        let existing = UsedBookOffer::find_by_id((book.bookmeter_id, site.as_str().to_string()))
            .one(&self.db)
            .await?;
        let known_product = existing
            .as_ref()
            .and_then(|m| m.product_id.as_deref().zip(m.product_url.as_deref()));
        let update = site.refresh_offer(isbn13, known_product).await?;
        if let Some(model) = existing {
            model
                .into_active_model()
                .apply_update(&update)
                .update(&self.db)
                .await?;
        } else {
            // 検索で商品が見つからなかった場合は行を作らず、次回実行時に再検索する
            if update.product_id.is_none() {
                return Ok(());
            }
            let mut active = used_book_offer::ActiveModel::from(&update);
            active.bookmeter_id = Set(book.bookmeter_id);
            active.site = Set(site.as_str().to_string());
            UsedBookOffer::insert(active).exec(&self.db).await?;
        }
        self.metrics.record_used_book_offer_fetched(site.as_str());
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_discounts(
        &self,
        limit: Option<u64>,
    ) -> Result<impl Stream<Item = Result<model::Model>> + '_> {
        Ok(Book::find()
            .filter(model::Column::Title.is_not_null())
            .filter(model::Column::BasisPrice.is_not_null())
            .filter(model::Column::Price.is_not_null())
            .filter(model::Column::DiscountRate.is_not_null())
            .order_by_desc(model::Column::DiscountRate)
            .order_by_desc(model::Column::Price)
            .order_by_asc(model::Column::Title)
            .limit(limit.unwrap_or(50))
            .stream(&self.db)
            .await
            .map_err(|e| anyhow::anyhow!("{e:?}"))?
            .map_err(|e| anyhow::anyhow!("{e:?}")))
    }
}
