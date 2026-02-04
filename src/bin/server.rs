use std::env;
use std::time::Duration;

use axum::{routing::get, Json, Router};
use bookmeter_discounts::model::Model as Book;
use bookmeter_discounts::BookMeterDiscounts;
use futures::TryStreamExt;
use sea_orm::{ConnectOptions, Database};
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    // ログ初期化
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("bookmeter_discounts=info".parse().unwrap()),
        )
        .init();

    info!("Starting server on 0.0.0.0:3000...");

    let app = Router::new().route("/", get(get_books));
    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

#[axum::debug_handler]
async fn get_books() -> Json<Vec<Book>> {
    let user_id = env::var("USER_ID").expect("USER_ID is not set");
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL is not set");

    // DB接続にタイムアウトを設定
    let mut opt = ConnectOptions::new(&database_url);
    opt.connect_timeout(Duration::from_secs(10))
        .acquire_timeout(Duration::from_secs(10));
    let db = Database::connect(opt).await.unwrap();
    let bookmeter_discounts_client = BookMeterDiscounts::new(&user_id, db, 0);
    let books = bookmeter_discounts_client
        .get_discounts(Some(100))
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    Json(books)
}
