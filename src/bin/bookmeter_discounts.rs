use std::env;

use bookmeter_discounts::BookMeterDiscounts;
use futures::TryStreamExt;
use sea_orm::Database;

#[tokio::main]
async fn main() {
    // メインの処理
    let user_id = env::var("USER_ID").expect("USER_ID must be set");
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db = Database::connect(&db_url).await.unwrap();
    let bookmeter_discounts = BookMeterDiscounts::new(&user_id, db);
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
            eprintln!("Error\t{:?}", e);
        }
    };

    // Webhookの送信
    if let Ok(url) = env::var("WEBHOOK_URL") {
        let client = reqwest::Client::new();
        let res = client.post(&url).send().await.unwrap();
        println!("Webhook\t{:?}", res);
    }
}
