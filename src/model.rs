use crate::bookmeter::BookMeterBook;
use sea_orm::{entity::prelude::*, Set};

#[derive(Clone, Debug, DeriveEntityModel)]
#[sea_orm(table_name = "books")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub bookmeter_id: i64,
    pub amazon_url: String,
    pub kindle_id: Option<String>,
    pub title: String,
    pub basis_price: Option<i32>,
    pub price: Option<i32>,
    pub discount_rate: Option<f32>,
    pub updated_at: chrono::NaiveDateTime,
}

impl ActiveModelBehavior for ActiveModel {}

#[derive(Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl From<BookMeterBook> for ActiveModel {
    fn from(bookmeter_book: BookMeterBook) -> ActiveModel {
        ActiveModel {
            bookmeter_id: Set(bookmeter_book.id as i64),
            amazon_url: Set(bookmeter_book.amazon_url),
            kindle_id: Set(None),
            title: Set(bookmeter_book.title),
            basis_price: Set(None),
            price: Set(None),
            discount_rate: Set(None),
            updated_at: Set(chrono::Utc::now().naive_utc()),
        }
    }
}
