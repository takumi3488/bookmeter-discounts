//! 中古本サイト (BOOKOFF・バリューブックス・ネットオフ) からの商品情報取得
//!
//! ISBN-13 で各サイトを検索して商品 ID / URL を特定し、
//! 商品ページから在庫・価格・状態を取得する。
//! パーサーは HTTP レスポンスの HTML 文字列を受け取る純粋関数として実装し、
//! `tests/fixtures/used_book/` の保存済み HTML でユニットテストできるようにする。

pub mod bookoff;
pub mod netoff;
pub mod valuebooks;

use std::time::Duration;

use anyhow::Result;

/// 対象の中古本サイト
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsedBookSite {
    Bookoff,
    ValueBooks,
    NetOff,
}

/// 検索結果 (商品 ID と商品ページ URL)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub product_id: String,
    pub product_url: String,
    /// 検索レスポンスがそのまま商品ページだった場合の解析結果 (バリューブックス用)
    pub details: Option<OfferDetails>,
}

/// 商品ページから取得するオファー情報
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfferDetails {
    /// 税込価格 (円)。在庫なしでも価格が分かる場合は Some
    pub price: Option<i32>,
    /// 商品状態 (例: "GOOD", "中古品")。サイトが状態を持たない場合は None
    pub condition: Option<String>,
    pub in_stock: bool,
}

/// DB 更新用の1サイト分の結果
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OfferUpdate {
    pub product_id: Option<String>,
    pub product_url: Option<String>,
    pub price: Option<i32>,
    pub condition: Option<String>,
    pub in_stock: bool,
}

impl UsedBookSite {
    pub const ALL: [UsedBookSite; 3] = [
        UsedBookSite::Bookoff,
        UsedBookSite::ValueBooks,
        UsedBookSite::NetOff,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            UsedBookSite::Bookoff => "bookoff",
            UsedBookSite::ValueBooks => "valuebooks",
            UsedBookSite::NetOff => "netoff",
        }
    }

    /// ISBN-13 で検索して商品 ID / URL を取得する
    ///
    /// 見つからなかった場合は `Ok(None)` を返す。
    ///
    /// # Errors
    ///
    /// HTTP リクエストまたは HTML の解析に失敗した場合にエラーを返す。
    pub async fn search(self, isbn13: &str) -> Result<Option<SearchHit>> {
        match self {
            UsedBookSite::Bookoff => bookoff::search(isbn13).await,
            UsedBookSite::ValueBooks => valuebooks::search(isbn13).await,
            UsedBookSite::NetOff => netoff::search(isbn13).await,
        }
    }

    /// 商品ページ URL から在庫・価格・状態を取得する
    ///
    /// # Errors
    ///
    /// HTTP リクエストまたは HTML の解析に失敗した場合にエラーを返す。
    pub async fn fetch_details(self, product_url: &str) -> Result<OfferDetails> {
        match self {
            UsedBookSite::Bookoff => bookoff::fetch_details(product_url).await,
            UsedBookSite::ValueBooks => valuebooks::fetch_details(product_url).await,
            UsedBookSite::NetOff => netoff::fetch_details(product_url).await,
        }
    }

    /// 既知の商品 ID / URL があればそれを、なければ ISBN 検索で、最新のオファー情報を取得する
    ///
    /// # Errors
    ///
    /// 既知 URL の取得に失敗した場合 (古いデータを消さないよう検索にはフォールバックしない)、
    /// または検索の HTTP リクエストに失敗した場合にエラーを返す。
    pub async fn refresh_offer(
        self,
        isbn13: &str,
        known_product: Option<(&str, &str)>,
    ) -> Result<OfferUpdate> {
        if let Some((product_id, product_url)) = known_product {
            let details = self.fetch_details(product_url).await?;
            return Ok(OfferUpdate {
                product_id: Some(product_id.to_string()),
                product_url: Some(product_url.to_string()),
                price: details.price,
                condition: details.condition,
                in_stock: details.in_stock,
            });
        }
        let Some(hit) = self.search(isbn13).await? else {
            return Ok(OfferUpdate::default());
        };
        let details = match hit.details {
            Some(details) => Some(details),
            None => match self.fetch_details(&hit.product_url).await {
                Ok(details) => Some(details),
                Err(e) => {
                    // 商品 ID / URL だけでも保持し、詳細は次回更新時に再取得する
                    tracing::warn!("failed to fetch details from {}: {:?}", hit.product_url, e);
                    None
                }
            },
        };
        Ok(OfferUpdate {
            product_id: Some(hit.product_id),
            product_url: Some(hit.product_url),
            price: details.as_ref().and_then(|d| d.price),
            condition: details.as_ref().and_then(|d| d.condition.clone()),
            in_stock: details.is_some_and(|d| d.in_stock),
        })
    }
}

/// 中古本サイト向けの共通 HTTP クライアントを組み立てる
///
/// ブラウザ UA を名乗らないと 403 を返すサイトがあるため、
/// `get-amazon-html.sh` と同様の UA を付ける。
///
/// # Errors
///
/// クライアントの構築に失敗した場合にエラーを返す。
fn http_client() -> Result<reqwest::Client> {
    const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(USER_AGENT)
        .default_headers({
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                reqwest::header::ACCEPT_LANGUAGE,
                reqwest::header::HeaderValue::from_static("ja"),
            );
            headers
        })
        .build()?;
    Ok(client)
}

/// JSON-LD (`application/ld+json`) ブロックから最初に見つかったオファーを取り出す
///
/// BOOKOFF / バリューブックスの商品ページが埋め込む schema.org データ向け。
///
/// # Errors
///
/// 常に `Ok` を返す (見つからない場合は `Ok(None)`)。
fn parse_json_ld_offer(html: &str) -> Result<Option<(Option<i32>, bool)>> {
    use scraper::{Html, Selector};
    let doc = Html::parse_document(html);
    let selector = Selector::parse(r#"script[type="application/ld+json"]"#)
        .map_err(|e| anyhow::anyhow!("Failed to parse selector: {e:?}"))?;
    for script in doc.select(&selector) {
        let text: String = script.text().collect();
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let Some(offers) = json.get("offers") else {
            continue;
        };
        let offers = match offers {
            serde_json::Value::Array(arr) => arr.clone(),
            single => vec![single.clone()],
        };
        if let Some(offer) = offers.first() {
            let price = offer.get("price").and_then(serde_json::Value::as_i64);
            let in_stock = offer
                .get("availability")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|a| a.ends_with("InStock"));
            return Ok(Some((price.and_then(|p| i32::try_from(p).ok()), in_stock)));
        }
    }
    Ok(None)
}
