//! 中古本オファーの DB まわりの統合テスト
//!
//! `PostgreSQL` が必要なため CI では実行しない。実行する場合:
//!
//! ```sh
//! docker compose up -d postgres
//! DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres \
//!   cargo test --test used_book_db -- --ignored
//! ```

use anyhow::anyhow;
use bookmeter_discounts::model::Entity as Book;
use bookmeter_discounts::used_book::UsedBookSite;
use bookmeter_discounts::used_book_offer::Entity as UsedBookOffer;
use bookmeter_discounts::BookMeterDiscounts;
use sea_orm::{ActiveValue::Set, ColumnTrait, Database, EntityTrait, ModelTrait, QueryFilter};

const DATABASE_URL_ENV: &str = "DATABASE_URL";

/// 吾輩は猫である (文春文庫) の ISBN-13
const ISBN: &str = "9784167158057";

#[tokio::test]
#[ignore = "requires a PostgreSQL database and network access"]
async fn used_book_offer_roundtrip() -> anyhow::Result<()> {
    let db = Database::connect(
        std::env::var(DATABASE_URL_ENV)
            .map_err(|e| anyhow!("DATABASE_URL must be set to run this test: {e}"))?,
    )
    .await?;
    let app = BookMeterDiscounts::new("0", db.clone(), 0);

    // 対象の本を登録 (binding_name が「文庫」なので中古本オファー取得の対象)
    let bookmeter_id: i64 = 9_999_999_001;
    Book::insert(bookmeter_discounts::model::ActiveModel {
        bookmeter_id: Set(bookmeter_id),
        amazon_url: Set("https://www.amazon.co.jp/dp/4167158054".to_string()),
        kindle_id: Set(None),
        title: Set("吾輩は猫である".to_string()),
        basis_price: Set(None),
        price: Set(None),
        discount_rate: Set(None),
        is_kindle_unlimited: Set(false),
        updated_at: Set(chrono::Utc::now().naive_utc()),
        active_at: Set(None),
        binding_name: Set(Some("文庫".to_string())),
    })
    .exec(&db)
    .await?;
    let book = Book::find_by_id(bookmeter_id)
        .one(&db)
        .await?
        .ok_or_else(|| anyhow!("book should exist"))?;

    // 漫画・ライトノベル除外のクエリが対象の本を拾うこと
    let eligible = Book::find()
        .filter(bookmeter_discounts::model::Column::BindingName.is_not_null())
        .filter(
            bookmeter_discounts::model::Column::BindingName.is_not_in(["コミック", "ライトノベル"]),
        )
        .all(&db)
        .await?;
    assert!(eligible.iter().any(|b| b.bookmeter_id == bookmeter_id));

    // 実サイトからオファーを取得して保存
    app.update_used_book_offer(&book, UsedBookSite::Bookoff, ISBN)
        .await?;
    let offer = UsedBookOffer::find_by_id((bookmeter_id, "bookoff".to_string()))
        .one(&db)
        .await?
        .ok_or_else(|| anyhow!("offer should exist"))?;
    assert_eq!(offer.product_id.as_deref(), Some("0016731582"));

    // 既知の商品 URL を使った再取得でも product_id が維持されること
    app.update_used_book_offer(&book, UsedBookSite::Bookoff, ISBN)
        .await?;
    let offer = UsedBookOffer::find_by_id((bookmeter_id, "bookoff".to_string()))
        .one(&db)
        .await?
        .ok_or_else(|| anyhow!("offer should exist"))?;
    assert_eq!(offer.product_id.as_deref(), Some("0016731582"));

    // 本を削除するとオファーも cascade で削除されること
    book.delete(&db).await?;
    let deleted = UsedBookOffer::find_by_id((bookmeter_id, "bookoff".to_string()))
        .one(&db)
        .await?;
    assert!(deleted.is_none());
    Ok(())
}
