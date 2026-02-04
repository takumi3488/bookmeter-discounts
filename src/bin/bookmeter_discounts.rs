use std::env;
use std::time::Duration;

use bookmeter_discounts::BookMeterDiscounts;
use futures::TryStreamExt;
use sea_orm::{ConnectOptions, Database};
use tracing::{error, info};
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

    info!("Starting bookmeter_discounts...");

    // メインの処理
    let user_id = env::var("USER_ID").expect("USER_ID must be set");
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let get_amazon_page_interval = env::var("GET_AMAZON_PAGE_INTERVAL")
        .unwrap_or("10".to_string())
        .parse::<u64>()
        .expect("GET_AMAZON_PAGE_INTERVAL must be a number");

    // DB接続にタイムアウトを設定
    let mut opt = ConnectOptions::new(&db_url);
    opt.connect_timeout(Duration::from_secs(10))
        .acquire_timeout(Duration::from_secs(10));
    info!("Connecting to database...");
    let db = Database::connect(opt).await.unwrap();
    info!("Database connected");
    let bookmeter_discounts = BookMeterDiscounts::new(&user_id, db, get_amazon_page_interval);
    match bookmeter_discounts.update_and_get_discounts().await {
        Ok(mut stream) => {
            println!("Title\tURL\tDiscount Rate");
            while let Some(item) = stream.try_next().await.unwrap() {
                println!(
                    "{}\thttps://www.amazon.co.jp/dp/{}\t{}",
                    item.title,
                    item.kindle_id.unwrap(),
                    item.discount_rate.unwrap()
                );
            }
        }
        Err(e) => {
            error!("Error\t{:?}", e);
        }
    };

    // Webhookの送信
    if let Ok(url) = env::var("WEBHOOK_URL") {
        let client = reqwest::Client::new();
        let res = client.post(&url).send().await.unwrap();
        info!("Webhook\t{:?}", res);
    }
}
