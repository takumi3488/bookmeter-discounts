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
    let directive = "bookmeter_discounts=info"
        .parse()
        .unwrap_or_else(|_| tracing_subscriber::filter::Directive::default());
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(directive))
        .init();

    info!("Starting bookmeter_discounts...");

    // メインの処理
    let user_id = match env::var("USER_ID") {
        Ok(v) => v,
        Err(e) => {
            error!("USER_ID must be set: {e}");
            return;
        }
    };
    let db_url = match env::var("DATABASE_URL") {
        Ok(v) => v,
        Err(e) => {
            error!("DATABASE_URL must be set: {e}");
            return;
        }
    };
    let get_amazon_page_interval = match env::var("GET_AMAZON_PAGE_INTERVAL")
        .unwrap_or_else(|_| "10".to_string())
        .parse::<u64>()
    {
        Ok(v) => v,
        Err(e) => {
            error!("GET_AMAZON_PAGE_INTERVAL must be a number: {e}");
            return;
        }
    };

    // DB接続にタイムアウトを設定
    let mut opt = ConnectOptions::new(&db_url);
    opt.connect_timeout(Duration::from_secs(10))
        .acquire_timeout(Duration::from_secs(10));
    info!("Connecting to database...");
    let db = match Database::connect(opt).await {
        Ok(db) => db,
        Err(e) => {
            error!("Failed to connect to database: {e}");
            return;
        }
    };
    info!("Database connected");
    let bookmeter_discounts = BookMeterDiscounts::new(&user_id, db, get_amazon_page_interval);
    match bookmeter_discounts.update_and_get_discounts().await {
        Ok(mut stream) => {
            println!("Title\tURL\tDiscount Rate");
            loop {
                match stream.try_next().await {
                    Ok(Some(item)) => {
                        println!(
                            "{}\thttps://www.amazon.co.jp/dp/{}\t{}",
                            item.title,
                            item.kindle_id.as_deref().unwrap_or(""),
                            item.discount_rate.unwrap_or(0.0)
                        );
                    }
                    Ok(None) => break,
                    Err(e) => {
                        error!("Failed to get next item: {:?}", e);
                        break;
                    }
                }
            }
        }
        Err(e) => {
            error!("Error\t{:?}", e);
        }
    }

    // Webhookの送信
    if let Ok(url) = env::var("WEBHOOK_URL") {
        let client = reqwest::Client::new();
        match client.post(&url).send().await {
            Ok(res) => info!("Webhook\t{:?}", res),
            Err(e) => error!("Webhook failed: {:?}", e),
        }
    }
}
