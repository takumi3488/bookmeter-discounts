use std::collections::BTreeSet;
use std::time::Duration;

use crate::model as Book;
use anyhow::Result;
use backon::{ExponentialBuilder, Retryable};
use scraper::{Html, Selector};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QuerySelect};
use serde::Deserialize;
use tokio::time::sleep;
use tracing::{info, warn};

pub struct BookMeterClient {
    pub user_id: u32,
}

#[derive(Clone, Debug)]
pub struct BookMeterBook {
    pub id: u32,
    pub title: String,
    pub amazon_url: String,
    /// 書籍の形式 (コミック / ライトノベル / 文庫 / 新書 / 単行本 など)
    pub binding_name: Option<String>,
}

impl BookMeterBook {
    /// # Errors
    ///
    /// Returns an error if fetching the book title or Amazon URL fails.
    pub async fn from_id(id: u32) -> Result<BookMeterBook> {
        let doc = Self::get_book_page_with_retry(id).await?;
        let html = Html::parse_document(&doc);
        let title = Self::parse_title(&html, id)?;
        let binding_name = Self::parse_binding_name(&html);
        let amazon_url = Self::get_amazon_url(id).await?;
        Ok(BookMeterBook {
            id,
            title,
            amazon_url,
            binding_name,
        })
    }

    /// # Errors
    ///
    /// Returns an error if the HTTP request fails.
    async fn get_book_page(id: u32) -> Result<String> {
        let url = format!("https://bookmeter.com/books/{id}");
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;
        let doc = client.get(&url).send().await?.text().await?;
        Ok(doc)
    }

    /// 本ページを指数バックオフでリトライしながら取得する
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request keeps failing.
    async fn get_book_page_with_retry(id: u32) -> Result<String> {
        { || Self::get_book_page(id) }
            .retry(
                ExponentialBuilder::default()
                    .with_max_delay(Duration::from_hours(4))
                    .without_max_times(),
            )
            .sleep(tokio::time::sleep)
            .notify(|e, dur| {
                warn!("retrying after {:?} because {:?}", dur, e);
            })
            .await
    }

    /// # Errors
    ///
    /// Returns an error if the title element is not found.
    fn parse_title(html: &Html, id: u32) -> Result<String> {
        let selector = Selector::parse(".inner__title")
            .map_err(|e| anyhow::anyhow!("Failed to parse selector: {e:?}"))?;
        let title = html
            .select(&selector)
            .next()
            .ok_or_else(|| anyhow::anyhow!("title not found: id={id}"))?
            .text()
            .collect();
        Ok(title)
    }

    /// 本ページのHTMLから形式 (コミック / ライトノベル / 文庫 など) を取得する
    ///
    /// 形式の要素が見つからない場合や、内容が空の場合は `None` を返す。
    #[must_use]
    pub fn parse_binding_name(html: &Html) -> Option<String> {
        let selector = Selector::parse(".current-book-detail__binding-name").ok()?;
        html.select(&selector)
            .next()
            .map(|e| {
                e.text()
                    .collect::<String>()
                    .trim()
                    .trim_start_matches("形式：")
                    .to_string()
            })
            .filter(|name| !name.is_empty())
    }

    /// 既存の本の形式だけを取得する (`binding_name` 未保持の本の補完用)
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails after retries.
    pub async fn fetch_binding_name(id: u32) -> Result<Option<String>> {
        let doc = Self::get_book_page_with_retry(id).await?;
        let html = Html::parse_document(&doc);
        Ok(Self::parse_binding_name(&html))
    }

    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or no Amazon URL is found.
    async fn get_amazon_url(id: u32) -> Result<String> {
        let url = format!("https://bookmeter.com/api/v1/books/{id}/external_book_stores.json?");
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;
        let json: ExternalBookStores = client.get(&url).send().await?.json().await?;
        for store in json.resources {
            if store.url.contains("amazon") {
                let url = store.url.trim().trim_matches('\'');
                return Ok(url.to_string());
            }
        }
        Err(anyhow::anyhow!("Amazon URL not found"))
    }
}

#[derive(Deserialize)]
pub struct ExternalBookStore {
    url: String,
}

#[derive(Deserialize)]
pub struct ExternalBookStores {
    resources: Vec<ExternalBookStore>,
}

impl BookMeterClient {
    #[must_use]
    pub fn new(user_id: u32) -> BookMeterClient {
        BookMeterClient { user_id }
    }

    /// 読書メーターのウィッシュリストにある本IDを全て取得する
    ///
    /// # Errors
    ///
    /// Returns an error if fetching pages fails.
    pub async fn fetch_wishlist_ids(&self, max_page: u16) -> Result<BTreeSet<i64>> {
        let mut book_ids = BTreeSet::new();
        let mut page = 1;
        while page <= max_page {
            let html = self.get_book_page_html(page).await?;
            let new_book_ids = BookMeterClient::get_book_ids_from_html(&html)?;
            if new_book_ids.is_empty() {
                break;
            }
            book_ids.extend(new_book_ids.into_iter().map(i64::from));
            page += 1;
            sleep(Duration::from_secs(1)).await;
        }
        Ok(book_ids)
    }

    /// 与えられたIDのうちDBに未登録のものだけ詳細を取得する
    ///
    /// # Errors
    ///
    /// Returns an error if querying the database fails or an ID overflows `u32`.
    pub async fn fetch_new_books(
        &self,
        wishlist_ids: &BTreeSet<i64>,
        db: &DatabaseConnection,
    ) -> Result<Vec<BookMeterBook>> {
        if wishlist_ids.is_empty() {
            return Ok(Vec::new());
        }
        let existing: BTreeSet<i64> = Book::Entity::find()
            .filter(Book::Column::BookmeterId.is_in(wishlist_ids.iter().copied()))
            .select_only()
            .column(Book::Column::BookmeterId)
            .into_tuple::<i64>()
            .all(db)
            .await?
            .into_iter()
            .collect();

        let mut book_results = Vec::new();
        for &id in wishlist_ids.difference(&existing) {
            let book_id = u32::try_from(id)?;
            let book = BookMeterBook::from_id(book_id).await;
            info!("got book_meter_book: {:?}", book);
            book_results.push(book);
            sleep(Duration::from_secs(1)).await;
        }
        Ok(book_results.into_iter().filter_map(Result::ok).collect())
    }

    /// 読書メーターの本IDをHTMLから取得する
    ///
    /// 結果は`BTreeSet`の`Result`で返す
    ///
    /// # Errors
    ///
    /// Returns an error if the selector cannot be parsed or a book ID cannot be parsed.
    fn get_book_ids_from_html(html: &Html) -> Result<BTreeSet<u32>> {
        let selector = Selector::parse(".detail__title > a")
            .map_err(|e| anyhow::anyhow!("Failed to parse selector: {e:?}"))?;
        let mut book_ids = BTreeSet::new();
        for node in html.select(&selector) {
            let href = node
                .value()
                .attr("href")
                .ok_or_else(|| anyhow::anyhow!("href attribute not found"))?;
            let id = href
                .split('/')
                .next_back()
                .ok_or_else(|| anyhow::anyhow!("Invalid href"))?
                .parse()?;
            book_ids.insert(id);
        }
        Ok(book_ids)
    }

    /// 読書メーターの読みたい本リストの指定したページのHTMLを取得する
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails.
    pub async fn get_book_page_html(&self, page: u16) -> Result<Html> {
        let url: String = format!(
            "https://bookmeter.com/users/{}/books/wish?page={page}",
            self.user_id
        );
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;
        let doc = client.get(&url).send().await?.text().await?;
        let html = Html::parse_document(&doc);
        Ok(html)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_binding_name_from_fragment(fragment: &str) -> Option<String> {
        let html = Html::parse_fragment(fragment);
        BookMeterBook::parse_binding_name(&html)
    }

    // 実際の本ページから抽出した .current-book-detail の断片
    #[test]
    fn test_parse_binding_name_light_novel() {
        let fragment = r#"
            <div class="current-book-detail">
              <p class="current-book-detail__binding-name">形式：ライトノベル</p>
              <p class="current-book-detail__publisher">出版社：スターツ出版</p>
            </div>
        "#;
        assert_eq!(
            parse_binding_name_from_fragment(fragment),
            Some("ライトノベル".to_string())
        );
    }

    #[test]
    fn test_parse_binding_name_comic() {
        let fragment = r#"
            <div class="current-book-detail">
              <p class="current-book-detail__binding-name">形式：コミック</p>
              <p class="current-book-detail__publisher">出版社：講談社</p>
            </div>
        "#;
        assert_eq!(
            parse_binding_name_from_fragment(fragment),
            Some("コミック".to_string())
        );
    }

    #[test]
    fn test_parse_binding_name_shinsho() {
        let fragment = r#"
            <div class="current-book-detail">
              <p class="current-book-detail__binding-name">形式：新書</p>
              <p class="current-book-detail__publisher">出版社：スターツ出版</p>
            </div>
        "#;
        assert_eq!(
            parse_binding_name_from_fragment(fragment),
            Some("新書".to_string())
        );
    }

    #[test]
    fn test_parse_binding_name_not_found() {
        let fragment = r#"<div class="current-book-detail"></div>"#;
        assert_eq!(parse_binding_name_from_fragment(fragment), None);
    }

    #[test]
    fn test_parse_binding_name_empty() {
        // 要素はあるが内容が空の場合は未取得 (None) として扱う
        let fragment = r#"
            <div class="current-book-detail">
              <p class="current-book-detail__binding-name"></p>
            </div>
        "#;
        assert_eq!(parse_binding_name_from_fragment(fragment), None);
    }
}
