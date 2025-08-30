use std::env;

use axum::{routing::get, Json, Router};
use bookmeter_discounts::model::Model as Book;
use bookmeter_discounts::BookMeterDiscounts;
use futures::TryStreamExt;
use sea_orm::Database;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let app = Router::new().route("/", get(get_books));
    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

#[axum::debug_handler]
async fn get_books() -> Json<Vec<Book>> {
    let user_id = env::var("USER_ID").expect("USER_ID is not set");
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL is not set");
    let db = Database::connect(&database_url).await.unwrap();
    let bookmeter_discounts_client = BookMeterDiscounts::new(&user_id, db, 0).unwrap();
    let books = bookmeter_discounts_client
        .get_discounts(Some(100))
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    Json(books)
}
