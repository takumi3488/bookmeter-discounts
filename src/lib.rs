use std::{env, time::Duration};

use anyhow::Result;
use bookmeter::BookMeterClient;

mod bookmeter;
mod kindle;
mod model;
use futures::{Stream, TryStreamExt};
use kindle::Kindle;
use model::Entity as Book;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
};
use tokio::time::sleep;

pub struct BookMeterDiscounts {
    pub user_id: String,
    pub db: DatabaseConnection,
}

impl BookMeterDiscounts {
    pub fn new(user_id: &str, db: DatabaseConnection) -> Self {
        Self {
            user_id: user_id.to_string(),
            db,
        }
    }

    #[allow(clippy::needless_lifetimes)]
    pub async fn update_and_get_discounts<'a>(
        &'a self,
    ) -> Result<impl Stream<Item = Result<model::Model>> + 'a> {
        self.update_discounts().await?;
        self.get_discounts().await
    }

    pub async fn update_discounts(&self) -> Result<()> {
        // 読書メーターから本情報の取得
        let max_page = env::var("MAX_PAGE").unwrap_or("1".to_string()).parse()?;
        let bookmeter_client = BookMeterClient::new(self.user_id.parse()?);
        let bookmeter_books = bookmeter_client.get_books(max_page, &self.db).await?;
        for bookmeter_book in bookmeter_books {
            let book = model::ActiveModel::from(bookmeter_book);
            if let Err(e) = Book::insert(book).exec(&self.db).await {
                eprintln!("{:?}", e);
            }
        }

        // kindle idの取得
        let mut stream = Book::find()
            .filter(model::Column::KindleId.is_null())
            .stream(&self.db)
            .await?;
        while let Some(item) = stream.try_next().await? {
            sleep(Duration::from_secs(1)).await;
            let mut book: model::ActiveModel = item.into();
            let amazon_url = book.amazon_url.clone().into_value().unwrap().to_string();
            let kindle_id = match Kindle::convert_amazon_url_to_kindle_id(&amazon_url).await {
                Ok(id) => id,
                Err(e) => {
                    eprintln!("{:?}", e);
                    continue;
                }
            };
            book.kindle_id = Set(Some(kindle_id));
            book.updated_at = Set(chrono::Utc::now().naive_utc());
            book.update(&self.db).await?;
        }

        // kindle id取得済みの本の価格を取得
        let mut stream = Book::find()
            .filter(model::Column::KindleId.is_not_null())
            .stream(&self.db)
            .await?;
        while let Some(item) = stream.try_next().await? {
            sleep(Duration::from_secs(1)).await;
            let mut book: model::ActiveModel = item.into();
            let kindle_id = book.kindle_id.clone().into_value().unwrap().to_string();
            let kindle = match Kindle::from_id(&kindle_id).await {
                Ok(kindle) => kindle,
                Err(e) => {
                    eprintln!("{:?}", e);
                    continue;
                }
            };
            book.basis_price = Set(Some(kindle.basis_price as i32));
            book.price = Set(Some(kindle.price as i32));
            book.discount_rate = Set(Some(kindle.discount_rate));
            book.update(&self.db).await?;
        }

        Ok(())
    }

    #[allow(clippy::needless_lifetimes)]
    pub async fn get_discounts<'a>(
        &'a self,
    ) -> Result<impl Stream<Item = Result<model::Model>> + 'a> {
        Ok(Book::find()
            .filter(model::Column::Title.is_not_null())
            .filter(model::Column::BasisPrice.is_not_null())
            .filter(model::Column::Price.is_not_null())
            .filter(model::Column::DiscountRate.is_not_null())
            .order_by_desc(model::Column::DiscountRate)
            .order_by_desc(model::Column::Price)
            .order_by_asc(model::Column::Title)
            .stream(&self.db)
            .await
            .map_err(|e| anyhow::anyhow!("{:?}", e))?
            .map_err(|e| anyhow::anyhow!("{:?}", e)))
    }
}
