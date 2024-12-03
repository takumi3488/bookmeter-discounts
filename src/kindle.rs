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

impl Kindle {
    /// AmazonのURLからIDを取得する
    fn convert_amazon_url_to_id(url: &str) -> Result<String> {
        let url = Url::parse(url.trim_matches('\'')).unwrap();
        let mut path_segments = url.path_segments().unwrap();
        let product_index = path_segments.clone().position(|s| s == "product");
        let dp_index = path_segments.clone().position(|s| s == "dp");
        if product_index.is_none() && dp_index.is_none() {
            Err(anyhow::anyhow!("Invalid URL"))
        } else {
            let index = product_index.unwrap_or(dp_index.unwrap());
            Ok(path_segments.nth(index + 1).unwrap().to_string())
        }
    }

    /// AmazonのURLからKindle IDを取得する
    pub async fn convert_amazon_url_to_kindle_id(url: &str) -> Result<String> {
        let id = Self::convert_amazon_url_to_id(url.trim_matches('\''))?;
        let doc = Kindle::get_html_by_amazon_id(&id).await?;
        let html = Html::parse_document(&doc);
        let selector =
            Selector::parse("#tmm-grid-swatch-KINDLE a.a-button-text.a-text-left").unwrap();
        let kindle_url = match html.select(&selector).next() {
            Some(node) => node.value().attr("href").unwrap(),
            None => return Err(anyhow::anyhow!("Kindle button not found: {}", url)),
        };
        if kindle_url == "javascript:void(0)" {
            Ok(id)
        } else {
            Ok(Self::convert_amazon_url_to_id(&format!(
                "https://www.amazon.co.jp{}",
                kindle_url
            ))?)
        }
    }

    /// AmazonのIDからHTMLを取得する
    pub async fn get_html_by_amazon_id(amazon_id: &str) -> Result<String> {
        const MAX_RETRY: u8 = 100;
        let mut count = 0;
        let curl_res = loop {
            match Command::new("bash")
                .args(["get-amazon-html.sh", amazon_id.trim_matches('\'')])
                .output()
                .await
            {
                Ok(res) => break res,
                Err(_) => {
                    count += 1;
                    if count >= MAX_RETRY {
                        return Err(anyhow::anyhow!("Failed to get HTML: {}", amazon_id));
                    }
                    sleep(Duration::from_secs(1)).await;
                }
            }
        };
        let html = String::from_utf8(curl_res.stdout)?;
        Ok(html)
    }

    /// KindleのIDから情報を取得する
    pub async fn from_id(kindle_id: &str) -> Result<Self> {
        let doc = reqwest::get(format!(
            "https://www.listasin.net/kndlsl/asins/{}",
            kindle_id.trim_matches('\'')
        ))
        .await?
        .text()
        .await?;
        let html = Html::parse_document(&doc);

        // 値段の取得
        let price_selector = Selector::parse(".item-price > span").unwrap();
        let price = html
            .select(&price_selector)
            .filter_map(|e| e.attr("data-price").map(|s| s.parse::<u32>().unwrap()))
            .next()
            .ok_or(anyhow::anyhow!("Price not found"))?;

        // 基本価格の取得
        let basis_selector = Selector::parse(".item-price > s").unwrap();
        let basis_price = html
            .select(&basis_selector)
            .next()
            .map(|element_ref| {
                element_ref
                    .text()
                    .collect::<String>()
                    .trim()
                    .parse::<u32>()
                    .unwrap()
            })
            .unwrap_or(price);

        // 還元ポイントの取得
        let point_selector = Selector::parse(".item-point > span").unwrap();
        let point = html
            .select(&point_selector)
            .filter_map(|e| e.attr("data-point").map(|s| s.parse::<u32>().unwrap()))
            .next()
            .unwrap_or(0);

        // 割引率の計算
        let discount_rate = (1.0 - (price - point) as f64 / basis_price as f64) as f32;

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
    fn test_convert_amazon_url_to_id() {
        let url = "https://www.amazon.co.jp/dp/product/4088843142/ref=as_li_tf_tl?camp=247&creative=1211&creativeASIN=4088843142&ie=UTF8&linkCode=as2&tag=bookmeter_book_middle_detail_pc_login-22";
        let kindle_id = Kindle::convert_amazon_url_to_id(url).unwrap();
        assert_eq!(&kindle_id, "4088843142");

        let url = "https://www.amazon.co.jp/dp/4088843142";
        let kindle_id = Kindle::convert_amazon_url_to_id(url).unwrap();
        assert_eq!(&kindle_id, "4088843142");

        let url = "https://www.amazon.co.jp/ONE-PIECE-%E3%83%A2%E3%83%8E%E3%82%AF%E3%83%AD%E7%89%88-110-%E3%82%B8%E3%83%A3%E3%83%B3%E3%83%97%E3%82%B3%E3%83%9F%E3%83%83%E3%82%AF%E3%82%B9DIGITAL-ebook/dp/B0DJB4QN8R/ref=tmm_kin_swatch_0?_encoding=UTF8&qid=&sr=";
        let kindle_id = Kindle::convert_amazon_url_to_id(url).unwrap();
        assert_eq!(&kindle_id, "B0DJB4QN8R");
    }
}
