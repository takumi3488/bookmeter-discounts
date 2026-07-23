//! 実際の中古本サイトにアクセスするライブテスト
//!
//! セレクタが実サイトの HTML と乖離していないかを手元で確認するためのもので、
//! CI では実行しない。実行する場合:
//!
//! ```sh
//! cargo test --test used_book_live -- --ignored
//! ```

use anyhow::anyhow;
use bookmeter_discounts::used_book::UsedBookSite;

/// 海に願いを風に祈りをそして君に誓いを (スターツ出版文庫) の ISBN-13
const ISBN: &str = "9784813705185";

#[tokio::test]
#[ignore = "hits real websites"]
async fn bookoff_live() -> anyhow::Result<()> {
    let hit = UsedBookSite::Bookoff
        .search(ISBN)
        .await?
        .ok_or_else(|| anyhow!("BOOKOFF search should find the book"))?;
    assert_eq!(hit.product_id, "0019117467");
    let details = UsedBookSite::Bookoff
        .fetch_details(&hit.product_url)
        .await?;
    assert!(details.price.is_some());
    Ok(())
}

#[tokio::test]
#[ignore = "hits real websites"]
async fn valuebooks_live() -> anyhow::Result<()> {
    let hit = UsedBookSite::ValueBooks
        .search(ISBN)
        .await?
        .ok_or_else(|| anyhow!("ValueBooks search should find the book"))?;
    assert!(hit.product_id.starts_with("VS"));
    let details = hit
        .details
        .ok_or_else(|| anyhow!("ValueBooks search parses details inline"))?;
    assert!(details.price.is_some() || !details.in_stock);
    Ok(())
}

#[tokio::test]
#[ignore = "hits real websites"]
async fn netoff_live() -> anyhow::Result<()> {
    let hit = UsedBookSite::NetOff
        .search(ISBN)
        .await?
        .ok_or_else(|| anyhow!("NetOff search should find the book"))?;
    assert_eq!(hit.product_id, "0012822282");
    let details = UsedBookSite::NetOff.fetch_details(&hit.product_url).await?;
    assert!(details.price.is_some());
    Ok(())
}

#[tokio::test]
#[ignore = "hits real websites"]
async fn refresh_offer_with_known_product_live() -> anyhow::Result<()> {
    let update = UsedBookSite::Bookoff
        .refresh_offer(
            ISBN,
            Some((
                "0019117467",
                "https://shopping.bookoff.co.jp/used/0019117467",
            )),
        )
        .await?;
    assert_eq!(update.product_id.as_deref(), Some("0019117467"));
    assert!(update.price.is_some());
    Ok(())
}
