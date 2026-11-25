use std::env;
use std::time::Duration;

use bookmeter_discounts::schedule::{run_daily, SystemClock};
use bookmeter_discounts::BookMeterDiscounts;
use chrono::NaiveTime;
use futures::TryStreamExt;
use sea_orm::{ConnectOptions, Database};
use tokio::signal::unix::{signal, SignalKind};
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
    // 設定時は常駐し、毎日この UTC 時刻 (HH:MM) に1回ずつ直列実行する。未設定時は1回実行して終了する
    let run_daily_at = match env::var("RUN_DAILY_AT_UTC")
        .ok()
        .map(|v| NaiveTime::parse_from_str(&v, "%H:%M"))
    {
        None => None,
        Some(Ok(at)) => Some(at),
        Some(Err(e)) => {
            error!("RUN_DAILY_AT_UTC must be HH:MM: {e}");
            std::process::exit(1);
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

    let Some(at) = run_daily_at else {
        // 失敗も従来どおりログのみで終了コード0
        if let Err(e) = run_once(&bookmeter_discounts).await {
            error!("Error\t{:?}", e);
        }
        return;
    };
    let mut sigterm = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to install SIGTERM handler: {e}");
            std::process::exit(1);
        }
    };
    let shutdown = async {
        tokio::select! {
            _ = sigterm.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
    };
    run_daily(&SystemClock, at, shutdown, || {
        run_once(&bookmeter_discounts)
    })
    .await;
    info!("Stopped");
}

/// 更新・結果出力・Webhook送信を1回行う。Webhookは更新の成否によらず送信する
async fn run_once(bookmeter_discounts: &BookMeterDiscounts) -> anyhow::Result<()> {
    let result = print_discounts(bookmeter_discounts).await;

    // Webhookの送信
    if let Ok(url) = env::var("WEBHOOK_URL") {
        let client = reqwest::Client::new();
        match client.post(&url).send().await {
            Ok(res) => info!("Webhook\t{:?}", res),
            Err(e) => error!("Webhook failed: {:?}", e),
        }
    }
    result
}

async fn print_discounts(bookmeter_discounts: &BookMeterDiscounts) -> anyhow::Result<()> {
    let mut stream = bookmeter_discounts.update_and_get_discounts().await?;
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
            Ok(None) => return Ok(()),
            Err(e) => {
                error!("Failed to get next item: {:?}", e);
                return Ok(());
            }
        }
    }
}
