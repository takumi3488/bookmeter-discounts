//! バリューブックスの検索・商品ページパーサー
//!
//! - 検索: `https://www.valuebooks.jp/search?keyword={isbn13}`
//!   ヒットした場合は商品ページ (`/bp/{product_id}`) にリダイレクトされる。
//!   ヒットしない場合は `/search` に留まる。
//! - 商品ページ: Vue の `<router-view :item-info="{...}">` に
//!   状態 (condition) ごとの価格・在庫を持つ JSON が埋め込まれている。

use anyhow::Result;
use scraper::{Html, Selector};
use serde::Deserialize;

use super::{http_client, OfferDetails, SearchHit};

const BASE_URL: &str = "https://www.valuebooks.jp";

/// ISBN-13 で検索して商品を取得する
///
/// 検索は商品ページへリダイレクトされるため、レスポンス HTML をそのまま解析し
/// `details` まで埋めて返す。ヒットしない場合は `Ok(None)` を返す。
///
/// # Errors
///
/// HTTP リクエストまたは HTML の解析に失敗した場合にエラーを返す。
pub async fn search(isbn13: &str) -> Result<Option<SearchHit>> {
    let url = format!("{BASE_URL}/search?keyword={isbn13}");
    let response = http_client()?.get(&url).send().await?;
    let final_url = response.url().to_string();
    let html = response.text().await?;
    let Some(product_id) = product_id_from_url(&final_url) else {
        return Ok(None);
    };
    Ok(Some(SearchHit {
        product_id,
        product_url: final_url,
        details: parse_product(&html).ok(),
    }))
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

/// 商品ページ URL (`.../bp/VS0051080734`) から商品 ID を取り出す
#[must_use]
pub fn product_id_from_url(url: &str) -> Option<String> {
    let marker = "/bp/";
    let start = url.rfind(marker)? + marker.len();
    let id: String = url[start..]
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

/// `:item-info` に埋め込まれる状態ごとの在庫情報
#[derive(Debug, Deserialize)]
struct ItemInfo {
    #[serde(default, rename = "genpinList")]
    genpin_list: Vec<Genpin>,
}

#[derive(Debug, Deserialize)]
struct Genpin {
    #[serde(rename = "conditionName")]
    condition_name: String,
    price: Option<i32>,
    #[serde(default)]
    stock: i64,
}

/// 商品ページ HTML から価格・在庫・状態を取り出す
///
/// 在庫のある状態のうち最安のものを `price` / `condition` とする。
///
/// # Errors
///
/// ページ構造の解析に失敗した場合にエラーを返す。
pub fn parse_product(html: &str) -> Result<OfferDetails> {
    let doc = Html::parse_document(html);
    let selector = Selector::parse("router-view")
        .map_err(|e| anyhow::anyhow!("Failed to parse selector: {e:?}"))?;
    let item_info = doc
        .select(&selector)
        .next()
        .and_then(|e| e.value().attr(":item-info"))
        .and_then(|json| serde_json::from_str::<ItemInfo>(json).ok());

    if let Some(item_info) = item_info {
        // 在庫のある状態のうち最安のものを選ぶ
        let cheapest = item_info
            .genpin_list
            .iter()
            .filter(|g| g.stock > 0 && g.price.is_some())
            .min_by_key(|g| g.price);
        return Ok(match cheapest {
            Some(genpin) => OfferDetails {
                price: genpin.price,
                condition: Some(genpin.condition_name.clone()),
                in_stock: true,
            },
            None => OfferDetails {
                price: None,
                condition: None,
                in_stock: false,
            },
        });
    }

    // :item-info が取れない場合は JSON-LD にフォールバック
    if let Some((price, in_stock)) = super::parse_json_ld_offer(html)? {
        return Ok(OfferDetails {
            price,
            condition: None,
            in_stock,
        });
    }
    Err(anyhow::anyhow!("Failed to parse ValueBooks product page"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // fixtures (tests/fixtures/used_book/):
    // - valuebooks_product.html: /bp/VS0051080734 (GOOD 287円・在庫あり)
    //   ※ ISBN 9784813705185 の検索からリダイレクトされた商品ページ
    // - valuebooks_product_oos.html: /bp/VS0040563019 (全状態在庫なし)
    // - valuebooks_search_empty.html: ヒットなし (検索ページに留まる)

    #[test]
    fn test_product_id_from_url() {
        assert_eq!(
            product_id_from_url(
                "https://www.valuebooks.jp/%E6%B5%B7%E3%81%AB%E9%A1%98%E3%81%84%E3%82%92--%E3%82%B9%E3%82%BF%E3%83%BC%E3%83%84%E5%87%BA%E7%89%88%E6%96%87%E5%BA%AB-/bp/VS0051080734"
            ),
            Some("VS0051080734".to_string())
        );
        assert_eq!(
            product_id_from_url("https://www.valuebooks.jp/search?keyword=9784000000000"),
            None
        );
    }

    #[test]
    fn test_parse_product_in_stock() -> Result<()> {
        let html = include_str!("../../tests/fixtures/used_book/valuebooks_product.html");
        let details = parse_product(html)?;
        assert_eq!(details.price, Some(287));
        assert_eq!(details.condition, Some("GOOD".to_string()));
        assert!(details.in_stock);
        Ok(())
    }

    #[test]
    fn test_parse_product_out_of_stock() -> Result<()> {
        let html = include_str!("../../tests/fixtures/used_book/valuebooks_product_oos.html");
        let details = parse_product(html)?;
        assert_eq!(details.price, None);
        assert_eq!(details.condition, None);
        assert!(!details.in_stock);
        Ok(())
    }

    #[test]
    fn test_parse_search_empty_page_has_no_product_id() {
        // ヒットなしの場合は /search に留まり商品 ID を取れない
        assert_eq!(
            product_id_from_url("https://www.valuebooks.jp/search?keyword=9784000000000"),
            None
        );
    }
}
