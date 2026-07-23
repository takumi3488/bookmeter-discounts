//! BOOKOFF (ブックオフ公式オンラインストア) の検索・商品ページパーサー
//!
//! - 検索: `https://shopping.bookoff.co.jp/search/keyword/{isbn13}` (サーバサイドレンダリング)
//! - 商品ページ: `https://shopping.bookoff.co.jp/used/{product_id}` (JSON-LD 埋め込みあり)

use anyhow::Result;
use scraper::{Html, Selector};

use super::{http_client, OfferDetails, SearchHit};

const BASE_URL: &str = "https://shopping.bookoff.co.jp";

/// ISBN-13 で検索して最初の商品を取得する
///
/// # Errors
///
/// HTTP リクエストまたは HTML の解析に失敗した場合にエラーを返す。
pub async fn search(isbn13: &str) -> Result<Option<SearchHit>> {
    let url = format!("{BASE_URL}/search/keyword/{isbn13}");
    let html = http_client()?.get(&url).send().await?.text().await?;
    parse_search(&html)
}

/// 商品ページから在庫・価格を取得する
///
/// # Errors
///
/// HTTP リクエストまたは HTML の解析に失敗した場合にエラーを返す。
pub async fn fetch_details(product_url: &str) -> Result<OfferDetails> {
    let html = http_client()?.get(product_url).send().await?.text().await?;
    parse_product(&html)
}

/// 検索結果 HTML から最初の商品の ID と URL を取り出す
///
/// 中古 (`/used/`) のリンクを優先し、なければ新品 (`/new/`) のリンクを使う。
/// ヒットなしの場合は `Ok(None)` を返す。
///
/// # Errors
///
/// セレクタの解析に失敗した場合にエラーを返す。
pub fn parse_search(html: &str) -> Result<Option<SearchHit>> {
    let doc = Html::parse_document(html);
    let item_selector = Selector::parse("a.productItem__link")
        .map_err(|e| anyhow::anyhow!("Failed to parse selector: {e:?}"))?;
    let mut used_href = None;
    let mut new_href = None;
    for node in doc.select(&item_selector) {
        let Some(href) = node.value().attr("href") else {
            continue;
        };
        if href.starts_with("/used/") && used_href.is_none() {
            used_href = Some(href.to_string());
        } else if href.starts_with("/new/") && new_href.is_none() {
            new_href = Some(href.to_string());
        }
    }
    let Some(href) = used_href.or(new_href) else {
        return Ok(None);
    };
    let product_id = href
        .rsplit('/')
        .next()
        .ok_or_else(|| anyhow::anyhow!("Invalid product href: {href}"))?
        .to_string();
    Ok(Some(SearchHit {
        product_id,
        product_url: format!("{BASE_URL}{href}"),
        details: None,
    }))
}

/// 商品ページ HTML から価格・在庫を取り出す
///
/// 埋め込み JSON-LD の `offers` を優先し、なければ HTML 要素から読む。
/// BOOKOFF は商品ごとの状態ランクを持たないため `condition` は `None`。
///
/// # Errors
///
/// 価格が取得できない場合にエラーを返す。
pub fn parse_product(html: &str) -> Result<OfferDetails> {
    if let Some((price, in_stock)) = super::parse_json_ld_offer(html)? {
        return Ok(OfferDetails {
            price,
            condition: None,
            in_stock,
        });
    }
    // JSON-LD が取れない場合のフォールバック
    let doc = Html::parse_document(html);
    let price_selector = Selector::parse(".productInformation__price--large")
        .map_err(|e| anyhow::anyhow!("Failed to parse selector: {e:?}"))?;
    let price = doc.select(&price_selector).next().and_then(|e| {
        e.text()
            .collect::<String>()
            .chars()
            .filter(char::is_ascii_digit)
            .collect::<String>()
            .parse::<i32>()
            .ok()
    });
    let stock_selector = Selector::parse(".productInformation__stock span")
        .map_err(|e| anyhow::anyhow!("Failed to parse selector: {e:?}"))?;
    let stock_text: String = doc
        .select(&stock_selector)
        .next()
        .map_or_else(String::new, |e| e.text().collect());
    let in_stock = stock_text.contains("在庫あり");
    if price.is_none() && !in_stock {
        return Err(anyhow::anyhow!("Failed to parse BOOKOFF product page"));
    }
    Ok(OfferDetails {
        price,
        condition: None,
        in_stock,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // fixtures (tests/fixtures/used_book/):
    // - bookoff_search.html: ISBN 9784813705185 の検索結果 (1件・在庫あり)
    // - bookoff_search_empty.html: 該当なしの検索結果
    // - bookoff_product.html: /used/0019117467 (495円・在庫あり)
    // - bookoff_product_oos.html: /used/0016731582 吾輩は猫である (220円・在庫なし)

    #[test]
    fn test_parse_search() -> Result<()> {
        let html = include_str!("../../tests/fixtures/used_book/bookoff_search.html");
        let hit = parse_search(html)?.ok_or_else(|| anyhow::anyhow!("hit expected"))?;
        assert_eq!(hit.product_id, "0019117467");
        assert_eq!(
            hit.product_url,
            "https://shopping.bookoff.co.jp/used/0019117467"
        );
        Ok(())
    }

    #[test]
    fn test_parse_search_empty() -> Result<()> {
        let html = include_str!("../../tests/fixtures/used_book/bookoff_search_empty.html");
        assert!(parse_search(html)?.is_none());
        Ok(())
    }

    #[test]
    fn test_parse_product_in_stock() -> Result<()> {
        let html = include_str!("../../tests/fixtures/used_book/bookoff_product.html");
        let details = parse_product(html)?;
        assert_eq!(details.price, Some(495));
        assert!(details.in_stock);
        assert_eq!(details.condition, None);
        Ok(())
    }

    #[test]
    fn test_parse_product_out_of_stock() -> Result<()> {
        let html = include_str!("../../tests/fixtures/used_book/bookoff_product_oos.html");
        let details = parse_product(html)?;
        assert_eq!(details.price, Some(220));
        assert!(!details.in_stock);
        Ok(())
    }
}
