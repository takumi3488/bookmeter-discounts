use std::time::Duration;

use anyhow::Result;
use scraper::{Html, Selector};
use tokio::{process::Command, time::sleep};
use url::Url;

pub struct Kindle {
    pub basis_price: u32,
    pub price: u32,
    pub discount_rate: f32,
}

#[derive(Debug)]
pub struct KindleEdition {
    pub kindle_id: String,
    pub is_kindle_unlimited: bool,
}

impl Kindle {
    /// `AmazonのURLからIDを取得する`
    ///
    /// # Errors
    ///
    /// Returns an error if the URL is invalid or does not contain a product/dp segment.
    pub(crate) fn convert_amazon_url_to_id(url: &str) -> Result<String> {
        let url = Url::parse(url.trim_matches('\''))?;
        let mut path_segments = url
            .path_segments()
            .ok_or_else(|| anyhow::anyhow!("Invalid URL: no path segments"))?;
        let product_index = path_segments.clone().position(|s| s == "product");
        let dp_index = path_segments.clone().position(|s| s == "dp");
        match product_index.or(dp_index) {
            None => Err(anyhow::anyhow!("Invalid URL")),
            Some(index) => path_segments
                .nth(index + 1)
                .map(ToString::to_string)
                .ok_or_else(|| anyhow::anyhow!("Invalid URL: missing segment after product/dp")),
        }
    }

    /// `AmazonのURLからKindle IDとKindle Unlimited対象かどうかを取得する`
    ///
    /// # Errors
    ///
    /// Returns an error if the URL is invalid or the Kindle button is not found.
    pub async fn convert_amazon_url_to_kindle_id(url: &str) -> Result<KindleEdition> {
        let id = Self::convert_amazon_url_to_id(url.trim_matches('\''))?;
        let doc = Kindle::get_html_by_amazon_id(&id).await?;
        Self::parse_kindle_edition(&doc, &id, url)
    }

    /// `Amazon商品ページのHTMLからKindle IDとKindle Unlimited対象かどうかを取得する`
    ///
    /// # Errors
    ///
    /// Returns an error if the Kindle button is not found or the Kindle URL is invalid.
    fn parse_kindle_edition(doc: &str, id: &str, url: &str) -> Result<KindleEdition> {
        let html = Html::parse_document(doc);
        let swatch_selector = Selector::parse("#tmm-grid-swatch-KINDLE")
            .map_err(|e| anyhow::anyhow!("Failed to parse selector: {e:?}"))?;
        let button_selector = Selector::parse("a.a-button-text.a-text-left")
            .map_err(|e| anyhow::anyhow!("Failed to parse selector: {e:?}"))?;
        let ku_selector = Selector::parse("i.a-icon-kindle-unlimited")
            .map_err(|e| anyhow::anyhow!("Failed to parse selector: {e:?}"))?;
        // ボタンとKUアイコンは同一のswatch部分木から取得する
        let swatch = html
            .select(&swatch_selector)
            .next()
            .ok_or_else(|| anyhow::anyhow!("Kindle button not found: {url}"))?;
        let kindle_url = swatch
            .select(&button_selector)
            .next()
            .ok_or_else(|| anyhow::anyhow!("Kindle button not found: {url}"))?
            .value()
            .attr("href")
            .ok_or_else(|| anyhow::anyhow!("href attribute not found"))?;
        let is_kindle_unlimited = swatch.select(&ku_selector).next().is_some();
        let kindle_id = if kindle_url == "javascript:void(0)" {
            id.to_string()
        } else {
            Self::convert_amazon_url_to_id(&format!("https://www.amazon.co.jp{kindle_url}"))?
        };
        Ok(KindleEdition {
            kindle_id,
            is_kindle_unlimited,
        })
    }

    /// `AmazonのIDからHTMLを取得する`
    ///
    /// # Errors
    ///
    /// Returns an error if fetching HTML fails after maximum retries.
    pub async fn get_html_by_amazon_id(amazon_id: &str) -> Result<String> {
        const MAX_RETRY: u8 = 100;
        let mut count = 0;
        let curl_res = loop {
            if let Ok(res) = Command::new("bash")
                .args(["get-amazon-html.sh", amazon_id.trim_matches('\'')])
                .output()
                .await
            {
                break res;
            }
            count += 1;
            if count >= MAX_RETRY {
                return Err(anyhow::anyhow!("Failed to get HTML: {amazon_id}"));
            }
            sleep(Duration::from_secs(1)).await;
        };
        let html = String::from_utf8(curl_res.stdout)?;
        Ok(html)
    }

    /// `KindleのIDから情報を取得する`
    ///
    /// # Errors
    ///
    /// Returns an error if the price or Kindle data cannot be fetched.
    pub async fn from_id(kindle_id: &str) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;
        let doc = client
            .get(format!(
                "https://www.listasin.net/kndlsl/asins/{}",
                kindle_id.trim_matches('\'')
            ))
            .send()
            .await?
            .text()
            .await?;
        let html = Html::parse_document(&doc);

        // 値段の取得
        let price_selector = Selector::parse(".item-price > span")
            .map_err(|e| anyhow::anyhow!("Failed to parse selector: {e:?}"))?;
        let price = html
            .select(&price_selector)
            .find_map(|e| e.attr("data-price").and_then(|s| s.parse::<u32>().ok()))
            .ok_or_else(|| anyhow::anyhow!("Price not found"))?;

        // 基本価格の取得
        let basis_selector = Selector::parse(".item-price > s")
            .map_err(|e| anyhow::anyhow!("Failed to parse selector: {e:?}"))?;
        let basis_price = html
            .select(&basis_selector)
            .next()
            .map_or(Ok(price), |element_ref| {
                element_ref
                    .text()
                    .collect::<String>()
                    .trim()
                    .parse::<u32>()
                    .map_err(|e| anyhow::anyhow!("Failed to parse basis price: {e}"))
            })?;

        // 還元ポイントの取得
        let point_selector = Selector::parse(".item-point > span")
            .map_err(|e| anyhow::anyhow!("Failed to parse selector: {e:?}"))?;
        let point = html
            .select(&point_selector)
            .find_map(|e| e.attr("data-point").and_then(|s| s.parse::<u32>().ok()))
            .unwrap_or(0);

        // 割引率の計算
        #[expect(
            clippy::cast_possible_truncation,
            reason = "discount rate is always within [0, 1]"
        )]
        let discount_rate = 1.0_f32 - (f64::from(price - point) / f64::from(basis_price)) as f32;

        Ok(Kindle {
            basis_price,
            price,
            discount_rate,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_amazon_url_to_id() -> Result<()> {
        let url = "https://www.amazon.co.jp/dp/product/4088843142/ref=as_li_tf_tl?camp=247&creative=1211&creativeASIN=4088843142&ie=UTF8&linkCode=as2&tag=bookmeter_book_middle_detail_pc_login-22";
        let kindle_id = Kindle::convert_amazon_url_to_id(url)?;
        assert_eq!(&kindle_id, "4088843142");

        let url = "https://www.amazon.co.jp/dp/4088843142";
        let kindle_id = Kindle::convert_amazon_url_to_id(url)?;
        assert_eq!(&kindle_id, "4088843142");

        let url = "https://www.amazon.co.jp/ONE-PIECE-%E3%83%A2%E3%83%8E%E3%82%AF%E3%83%AD%E7%89%88-110-%E3%82%B8%E3%83%A3%E3%83%B3%E3%83%97%E3%82%B3%E3%83%9F%E3%83%83%E3%82%AF%E3%82%B9DIGITAL-ebook/dp/B0DJB4QN8R/ref=tmm_kin_swatch_0?_encoding=UTF8&qid=&sr=";
        let kindle_id = Kindle::convert_amazon_url_to_id(url)?;
        assert_eq!(&kindle_id, "B0DJB4QN8R");

        Ok(())
    }

    // 実際にKU対象の紙書籍ページ(dpページ)から抽出した #tmm-grid-swatch-KINDLE の断片。
    // href付きのKindleリンクと a-icon-kindle-unlimited アイコンを含む。
    const PAPER_KU_SWATCH_FRAGMENT: &str = r#"
        <div id="tmm-grid-swatch-KINDLE" class="a-column a-span6 a-text-left swatchElement unselected celwidget" role="listitem">
          <span class="a-button a-spacing-none a-button-toggle format kindleALCExtraMessage">
            <span class="a-button-inner">
              <a href="/%E6%B1%9A%E3%82%8C%E3%81%9F%E6%89%8B%E3%82%92%E3%81%9D%E3%81%93%E3%81%A7%E6%8B%AD%E3%81%8B%E3%81%AA%E3%81%84-%E6%96%87%E6%98%A5%E6%96%87%E5%BA%AB-%E8%8A%A6%E6%B2%A2-%E5%A4%AE-ebook/dp/B0CM5XVJ7Q/ref=tmm_kin_swatch_0" role="radio" aria-checked="false" aria-current="" class="a-button-text a-text-left">
                <span class="slot-title"><span aria-label="Kindle版 (電子書籍) 形式:">Kindle版 (電子書籍)</span></span>
                <span class="slot-price">
                  <span aria-label="￥0">￥0</span>
                  <i class="a-icon a-icon-kindle-unlimited a-icon-small" role="img" aria-label="Kindle Unlimitedで"></i>
                </span>
              </a>
            </span>
          </span>
        </div>
    "#;

    // 実際に非KUの紙書籍ページ(ONE PIECE 110)から抽出した #tmm-grid-swatch-KINDLE の断片。
    // href付きのKindleリンクを含むが、a-icon-kindle-unlimited アイコンは存在しない。
    const PAPER_NON_KU_SWATCH_FRAGMENT: &str = r#"
        <div id="tmm-grid-swatch-KINDLE" class="a-column a-span6 a-text-left swatchElement unselected celwidget" role="listitem">
          <span class="a-button a-spacing-none a-button-toggle format">
            <span class="a-button-inner">
              <a href="/ONE-PIECE-%E3%83%A2%E3%83%8E%E3%82%AF%E3%83%AD%E7%89%88-110-%E3%82%B8%E3%83%A3%E3%83%B3%E3%83%97%E3%82%B3%E3%83%9F%E3%83%83%E3%82%AF%E3%82%B9DIGITAL-ebook/dp/B0DJB4QN8R/ref=tmm_kin_swatch_0" role="radio" aria-checked="false" aria-current="" class="a-button-text a-text-left">
                <span class="slot-title"><span aria-label="Kindle版 (電子書籍) 形式:">Kindle版 (電子書籍)</span></span>
                <span class="slot-price"><span aria-label="￥543" class="a-size-base a-color-secondary ebook-price-value">￥543</span></span>
              </a>
            </span>
          </span>
        </div>
    "#;

    // 実際にKU対象のKindle本ページから抽出した #tmm-grid-swatch-KINDLE の断片。
    // 選択中の形式なので href="javascript:void(0)"。a-icon-kindle-unlimited アイコンを含む。
    const KINDLE_PAGE_KU_SWATCH_FRAGMENT: &str = r#"
        <div id="tmm-grid-swatch-KINDLE" class="a-column a-span6 a-text-left swatchElement selected celwidget" role="listitem">
          <span class="a-button a-button-selected a-spacing-none a-button-toggle format kindleALCExtraMessage">
            <span class="a-button-inner">
              <a href="javascript:void(0)" role="radio" aria-checked="true" aria-current="page" class="a-button-text a-text-left">
                <span class="slot-title"><span aria-label="Kindle版 (電子書籍) 形式:">Kindle版 (電子書籍)</span></span>
                <span class="slot-price">
                  <span aria-label="￥0" class="a-color-price">￥0</span>
                  <i class="a-icon a-icon-kindle-unlimited a-icon-small" role="img" aria-label="Kindle Unlimitedで"></i>
                </span>
              </a>
            </span>
          </span>
        </div>
    "#;

    // 実際に非KUのKindle本ページから抽出した #tmm-grid-swatch-KINDLE の断片。
    // 選択中の形式なので href="javascript:void(0)"。a-icon-kindle-unlimited アイコンは存在しない。
    const KINDLE_PAGE_NON_KU_SWATCH_FRAGMENT: &str = r#"
        <div id="tmm-grid-swatch-KINDLE" class="a-column a-span6 a-text-left swatchElement selected celwidget" role="listitem">
          <span class="a-button a-button-selected a-spacing-none a-button-toggle format">
            <span class="a-button-inner">
              <a href="javascript:void(0)" role="radio" aria-checked="true" aria-current="page" class="a-button-text a-text-left">
                <span class="slot-title"><span aria-label="Kindle版 (電子書籍) 形式:">Kindle版 (電子書籍)</span></span>
                <span class="slot-price"><span aria-label="￥543" class="a-size-base a-color-price a-color-price ebook-price-value">￥543</span></span>
              </a>
            </span>
          </span>
        </div>
    "#;

    // 非KUの紙書籍ページのナビゲーションから抽出した「Kindle Unlimited」文字列を含む要素と、
    // swatch外に置かれた a-icon-kindle-unlimited アイコン。誤検出防止の確認用。
    const OUTSIDE_SWATCH_KU_NOISE_FRAGMENT: &str = r#"
        <a href="/kindle-dbs/hz/signup/?_encoding=UTF8&ref_=sv_b_5" class="nav-a" aria-label="Kindle Unlimited 読み放題">
          <span class="nav-a-content">Kindle Unlimited 読み放題</span>
        </a>
        <i class="a-icon a-icon-kindle-unlimited a-icon-small" role="img" aria-label="Kindle Unlimitedで"></i>
    "#;

    #[test]
    fn test_parse_kindle_edition_paper_ku() -> Result<()> {
        let edition = Kindle::parse_kindle_edition(
            PAPER_KU_SWATCH_FRAGMENT,
            "4167921251",
            "https://www.amazon.co.jp/dp/4167921251",
        )?;
        assert_eq!(&edition.kindle_id, "B0CM5XVJ7Q");
        assert!(edition.is_kindle_unlimited);
        Ok(())
    }

    #[test]
    fn test_parse_kindle_edition_paper_non_ku() -> Result<()> {
        let edition = Kindle::parse_kindle_edition(
            PAPER_NON_KU_SWATCH_FRAGMENT,
            "4088843142",
            "https://www.amazon.co.jp/dp/4088843142",
        )?;
        assert_eq!(&edition.kindle_id, "B0DJB4QN8R");
        assert!(!edition.is_kindle_unlimited);
        Ok(())
    }

    #[test]
    fn test_parse_kindle_edition_kindle_page_ku() -> Result<()> {
        let edition = Kindle::parse_kindle_edition(
            KINDLE_PAGE_KU_SWATCH_FRAGMENT,
            "B0CM5XVJ7Q",
            "https://www.amazon.co.jp/dp/B0CM5XVJ7Q",
        )?;
        assert_eq!(&edition.kindle_id, "B0CM5XVJ7Q");
        assert!(edition.is_kindle_unlimited);
        Ok(())
    }

    #[test]
    fn test_parse_kindle_edition_kindle_page_non_ku() -> Result<()> {
        let edition = Kindle::parse_kindle_edition(
            KINDLE_PAGE_NON_KU_SWATCH_FRAGMENT,
            "B0DJB4QN8R",
            "https://www.amazon.co.jp/dp/B0DJB4QN8R",
        )?;
        assert_eq!(&edition.kindle_id, "B0DJB4QN8R");
        assert!(!edition.is_kindle_unlimited);
        Ok(())
    }

    #[test]
    fn test_parse_kindle_edition_button_not_found() {
        let doc = r#"<div id="tmm-grid-swatch-OTHER" class="a-column a-span6 a-text-left swatchElement selected celwidget"></div>"#;
        let url = "https://www.amazon.co.jp/dp/4088843142";
        match Kindle::parse_kindle_edition(doc, "4088843142", url) {
            Ok(edition) => panic!("expected an error but got {edition:?}"),
            Err(e) => assert!(e
                .to_string()
                .contains(&format!("Kindle button not found: {url}"))),
        }
    }

    #[test]
    fn test_parse_kindle_edition_ignores_ku_signals_outside_swatch() -> Result<()> {
        let doc = format!("{OUTSIDE_SWATCH_KU_NOISE_FRAGMENT}{PAPER_NON_KU_SWATCH_FRAGMENT}");
        let edition = Kindle::parse_kindle_edition(
            &doc,
            "4088843142",
            "https://www.amazon.co.jp/dp/4088843142",
        )?;
        assert_eq!(&edition.kindle_id, "B0DJB4QN8R");
        assert!(!edition.is_kindle_unlimited);
        Ok(())
    }

    #[test]
    fn test_parse_kindle_edition_uses_first_swatch_only() -> Result<()> {
        // 同じIDのswatchが複数あっても、ボタンとKUアイコンを同一(最初)のswatchから読むこと
        let doc = format!("{PAPER_NON_KU_SWATCH_FRAGMENT}{PAPER_KU_SWATCH_FRAGMENT}");
        let edition = Kindle::parse_kindle_edition(
            &doc,
            "4088843142",
            "https://www.amazon.co.jp/dp/4088843142",
        )?;
        assert_eq!(&edition.kindle_id, "B0DJB4QN8R");
        assert!(!edition.is_kindle_unlimited);
        Ok(())
    }
}
