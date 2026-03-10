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
    let directive = "bookmeter_discounts=info"
        .parse()
        .unwrap_or_else(|_| tracing_subscriber::filter::Directive::default());
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(directive))
        .init();

    info!("Starting server on 0.0.0.0:3000...");

    let app = Router::new().route("/", get(get_books));
    let listener = match TcpListener::bind("0.0.0.0:3000").await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to bind: {e}");
            return;
        }
    };
    if let Err(e) = axum::serve(listener, app).await {
        tracing::error!("Server error: {e}");
    }
}

#[axum::debug_handler]
async fn get_books() -> Json<Vec<Book>> {
    let user_id = env::var("USER_ID").unwrap_or_default();
    let database_url = env::var("DATABASE_URL").unwrap_or_default();

    // DB接続にタイムアウトを設定
    let mut opt = ConnectOptions::new(&database_url);
    opt.connect_timeout(Duration::from_secs(10))
        .acquire_timeout(Duration::from_secs(10));
    let db = match Database::connect(opt).await {
        Ok(db) => db,
        Err(e) => {
            tracing::error!("Failed to connect to database: {e}");
            return Json(Vec::new());
        }
    };
    let bookmeter_discounts_client = BookMeterDiscounts::new(&user_id, db, 0);
    let stream_result = bookmeter_discounts_client.get_discounts(Some(100)).await;
    match stream_result {
        Ok(stream) => match stream.try_collect::<Vec<_>>().await {
            Ok(books) => Json(books),
            Err(e) => {
                tracing::error!("Failed to collect books: {e:?}");
                Json(Vec::new())
            }
        },
        Err(e) => {
            tracing::error!("Failed to get discounts: {e:?}");
            Json(Vec::new())
        }
    }
}
