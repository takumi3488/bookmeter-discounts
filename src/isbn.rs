//! ISBN-10 から ISBN-13 への変換
//!
//! 中古本サイトの検索には ISBN-13 (JAN) を使うため、
//! Amazon の紙書籍 ASIN (= ISBN-10) を変換する。

use anyhow::Result;

/// ISBN-10 を ISBN-13 に変換する
///
/// 先頭に `978` を付け、チェックディジットを計算し直す。
/// 日本の書籍は全て `978-4` 始まりなので `978` 固定で問題ない。
///
/// # Errors
///
/// 入力が ISBN-10 の形式 (数字9桁 + 数字またはXのチェックディジット) でない場合にエラーを返す。
pub fn isbn10_to_isbn13(isbn10: &str) -> Result<String> {
    let isbn10 = isbn10.trim();
    let chars: Vec<char> = isbn10.chars().collect();
    let valid_length = chars.len() == 10;
    let valid_body = chars.iter().take(9).all(char::is_ascii_digit);
    let valid_check = chars
        .last()
        .is_some_and(|c| c.is_ascii_digit() || *c == 'X');
    if !(valid_length && valid_body && valid_check) {
        return Err(anyhow::anyhow!("Invalid ISBN-10: {isbn10}"));
    }
    let body12 = format!("978{}", &isbn10[..9]);
    let sum: u32 = body12
        .chars()
        .enumerate()
        .map(|(i, c)| {
            let digit = c.to_digit(10).unwrap_or(0);
            if i % 2 == 0 {
                digit
            } else {
                digit * 3
            }
        })
        .sum();
    let check = (10 - sum % 10) % 10;
    Ok(format!("{body12}{check}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_isbn10_to_isbn13() -> Result<()> {
        // 海に願いを風に祈りをそして君に誓いを (スターツ出版文庫)
        assert_eq!(isbn10_to_isbn13("4813705189")?, "9784813705185");
        // 吾輩は猫である (文春文庫)
        assert_eq!(isbn10_to_isbn13("4167158054")?, "9784167158057");
        // チェックディジットがXのケース
        assert_eq!(isbn10_to_isbn13("400000008X")?, "9784000000086");
        Ok(())
    }

    #[test]
    fn test_isbn10_to_isbn13_invalid() {
        // Kindle ASIN などは変換できない
        assert!(isbn10_to_isbn13("B0DJB4QN8R").is_err());
        assert!(isbn10_to_isbn13("481370518").is_err());
        assert!(isbn10_to_isbn13("48137051890").is_err());
        assert!(isbn10_to_isbn13("").is_err());
    }
}
