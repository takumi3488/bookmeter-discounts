use std::{env, time::Duration};

use anyhow::Result;
use bookmeter::BookMeterClient;

mod bookmeter;
mod kindle;
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
}

impl BookMeterDiscounts {
    pub fn new(user_id: &str, db: DatabaseConnection, get_amazon_page_interval: u64) -> Self {
        Self {
            user_id: user_id.to_string(),
            db,
            get_amazon_page_interval,
        }
    }

    #[allow(clippy::needless_lifetimes)]
    pub async fn update_and_get_discounts<'a>(
        &'a self,
    ) -> Result<impl Stream<Item = Result<model::Model>> + 'a> {
        self.update_discounts().await?;
        self.get_discounts(Some(10)).await
    }

    pub async fn update_discounts(&self) -> Result<()> {
        // 読書メーターから本情報の取得
        let max_page = env::var("MAX_PAGE").unwrap_or("1".to_string()).parse()?;
        let bookmeter_client = BookMeterClient::new(self.user_id.parse()?);
        let bookmeter_books = bookmeter_client.get_books(max_page, &self.db).await?;
        for bookmeter_book in bookmeter_books.clone() {
            let book = model::ActiveModel::from(bookmeter_book);
            if let Err(e) = Book::insert(book).exec(&self.db).await {
                eprintln!("{:?}", e);
            }
        }

        // 読書メーターから削除済みの本の削除
        let mut stream = Book::find().stream(&self.db).await?;
        while let Some(item) = stream.try_next().await? {
            let book: model::Model = item;
            let active_book = book.clone().into_active_model();
            if bookmeter_books
                .iter()
                .all(|b| b.id as i64 != book.bookmeter_id)
            {
                println!("delete book: {}", book.title);
                active_book.delete(&self.db).await?;
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
                println!("skip getting kindle id for {}", book.title,);
                continue;
            }
            sleep(Duration::from_secs(self.get_amazon_page_interval)).await;
            let kindle_id = match Kindle::convert_amazon_url_to_kindle_id(&book.amazon_url).await {
                Ok(id) => id,
                Err(e) => {
                    eprintln!(
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
            let kindle_id = book.kindle_id.clone().into_value().unwrap().to_string();
            let kindle = match Kindle::from_id(&kindle_id).await {
                Ok(kindle) => kindle,
                Err(e) => {
                    eprintln!(
                        "error while getting kindle price from {}: {:?}",
                        kindle_id, e
                    );
                    continue;
                }
            };
            book.basis_price = Set(Some(kindle.basis_price as i32));
            book.price = Set(Some(kindle.price as i32));
            book.discount_rate = Set(Some(kindle.discount_rate));
            book.updated_at = Set(chrono::Utc::now().naive_utc());
            book.update(&self.db).await?;
        }

        Ok(())
    }

    #[allow(clippy::needless_lifetimes)]
    pub async fn get_discounts<'a>(
        &'a self,
        limit: Option<u64>,
    ) -> Result<impl Stream<Item = Result<model::Model>> + 'a> {
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
            .map_err(|e| anyhow::anyhow!("{:?}", e))?
            .map_err(|e| anyhow::anyhow!("{:?}", e)))
    }
}
