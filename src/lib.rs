use std::{env, sync::Arc, time::Duration};

use anyhow::Result;
use bookmeter::BookMeterClient;
use tracing::{error, info};

mod bookmeter;
mod kindle;
mod metrics;
pub mod model;
use futures::{Stream, TryStreamExt};
use kindle::Kindle;
use model::Entity as Book;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, QuerySelect, Set,
};
use tokio::time::sleep;

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

        // kindle idの取得
        let mut stream = Book::find()
            .filter(model::Column::KindleId.is_null())
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
                info!("skip getting kindle id for {}", book.title,);
                continue;
            }
            sleep(Duration::from_secs(self.get_amazon_page_interval)).await;
            let kindle_id = match Kindle::convert_amazon_url_to_kindle_id(&book.amazon_url).await {
                Ok(id) => id,
                Err(e) => {
                    info!(
                        "error while getting kindle id from {}: {:?}",
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
            active_book.kindle_id = Set(Some(kindle_id));
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
