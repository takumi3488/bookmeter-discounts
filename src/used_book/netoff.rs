//! ネットオフの検索・商品ページパーサー
//!
//! - 検索: `https://www.netoff.co.jp/cmdtyallsearch/?cat=1002&word={isbn13}`
//!   (`cat=1002` は「古本・中古本」カテゴリ)
//! - 商品ページ: `https://www.netoff.co.jp/detail/{product_id}/`

use anyhow::Result;
use scraper::{Html, Selector};

use super::{http_client, OfferDetails, SearchHit};

const BASE_URL: &str = "https://www.netoff.co.jp";

/// ISBN-13 で検索して最初の商品を取得する
///
/// # Errors
///
/// HTTP リクエストまたは HTML の解析に失敗した場合にエラーを返す。
pub async fn search(isbn13: &str) -> Result<Option<SearchHit>> {
    let url = format!("{BASE_URL}/cmdtyallsearch/?cat=1002&word={isbn13}");
    let html = http_client()?.get(&url).send().await?.text().await?;
    parse_search(&html)
}

/// 商品ページから在庫・価格・状態を取得する
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
/// ヒットなしの場合は `Ok(None)` を返す。
///
/// # Errors
///
/// セレクタの解析に失敗した場合にエラーを返す。
pub fn parse_search(html: &str) -> Result<Option<SearchHit>> {
    let doc = Html::parse_document(html);
    let selector = Selector::parse("a.c-cassette__title")
        .map_err(|e| anyhow::anyhow!("Failed to parse selector: {e:?}"))?;
    let Some(href) = doc
        .select(&selector)
        .find_map(|e| e.value().attr("href").map(str::to_string))
    else {
        return Ok(None);
    };
    // href は "/detail/0012822282/" 形式
    let product_id = href
        .trim_matches('/')
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

/// 商品ページ HTML から価格・在庫・状態を取り出す
///
/// # Errors
///
/// 価格が取得できない場合にエラーを返す。
pub fn parse_product(html: &str) -> Result<OfferDetails> {
    let doc = Html::parse_document(html);

    let price_selector = Selector::parse(".product-price__normal-num")
        .map_err(|e| anyhow::anyhow!("Failed to parse selector: {e:?}"))?;
    let price = doc.select(&price_selector).next().and_then(|e| {
        e.text()
            .collect::<String>()
            .trim()
            .replace(',', "")
            .parse::<i32>()
            .ok()
    });

    // 在庫がある場合は「在庫あとN点！」などの文言が入る (ない場合は要素自体が空)
    let stock_selector = Selector::parse(".l-product__stock-text")
        .map_err(|e| anyhow::anyhow!("Failed to parse selector: {e:?}"))?;
    let in_stock = doc.select(&stock_selector).next().is_some();

    // 「状態：中古品」のような文言から状態を取り出す (在庫がない場合は要素自体がない)
    let condition_selector = Selector::parse(".l-product__condition-text")
        .map_err(|e| anyhow::anyhow!("Failed to parse selector: {e:?}"))?;
    let condition = doc.select(&condition_selector).next().map(|e| {
        e.text()
            .collect::<String>()
            .trim()
            .trim_start_matches("状態：")
            .to_string()
    });

    if price.is_none() {
        return Err(anyhow::anyhow!("Failed to parse NetOff product page"));
    }
    Ok(OfferDetails {
        price,
        condition,
        in_stock,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // fixtures (tests/fixtures/used_book/):
    // - netoff_search.html: ISBN 9784813705185 の検索結果 (1件)
    // - netoff_search_empty.html: 該当なしの検索結果
    // - netoff_product.html: /detail/0012822282/ (220円・在庫あり・状態：中古品)
    // - netoff_product_oos.html: /detail/0011631528/ (110円・在庫なし)

    #[test]
    fn test_parse_search() -> Result<()> {
        let html = include_str!("../../tests/fixtures/used_book/netoff_search.html");
        let hit = parse_search(html)?.ok_or_else(|| anyhow::anyhow!("hit expected"))?;
        assert_eq!(hit.product_id, "0012822282");
        assert_eq!(
            hit.product_url,
            "https://www.netoff.co.jp/detail/0012822282/"
        );
        Ok(())
    }

    #[test]
    fn test_parse_search_empty() -> Result<()> {
        let html = include_str!("../../tests/fixtures/used_book/netoff_search_empty.html");
        assert!(parse_search(html)?.is_none());
        Ok(())
    }

    #[test]
    fn test_parse_product_in_stock() -> Result<()> {
        let html = include_str!("../../tests/fixtures/used_book/netoff_product.html");
        let details = parse_product(html)?;
        assert_eq!(details.price, Some(220));
        assert_eq!(details.condition, Some("中古品".to_string()));
        assert!(details.in_stock);
        Ok(())
    }

    #[test]
    fn test_parse_product_out_of_stock() -> Result<()> {
        let html = include_str!("../../tests/fixtures/used_book/netoff_product_oos.html");
        let details = parse_product(html)?;
        assert_eq!(details.price, Some(110));
        assert_eq!(details.condition, None);
        assert!(!details.in_stock);
        Ok(())
    }
}
