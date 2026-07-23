use sea_orm::{entity::prelude::*, ActiveValue::Set};
use serde::{Deserialize, Serialize};

use crate::used_book::OfferUpdate;

/// 中古本サイト (bookoff / valuebooks / netoff) の商品オファー
#[derive(Clone, Debug, DeriveEntityModel, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[sea_orm(table_name = "used_book_offers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub bookmeter_id: i64,
    /// `UsedBookSite::as_str()` の値 (bookoff / valuebooks / netoff)
    #[sea_orm(primary_key, auto_increment = false)]
    pub site: String,
    pub product_id: Option<String>,
    pub product_url: Option<String>,
    /// 税込価格 (円)
    pub price: Option<i32>,
    /// 商品状態 (バリューブックスの GOOD など)。サイトが状態を持たない場合は None
    pub condition: Option<String>,
    pub in_stock: bool,
    pub updated_at: chrono::NaiveDateTime,
}

impl ActiveModelBehavior for ActiveModel {}

#[derive(Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModel {
    /// オファー情報の更新結果を `ActiveModel` に反映する
    #[must_use]
    pub fn apply_update(mut self, update: &OfferUpdate) -> Self {
        self.product_id = Set(update.product_id.clone());
        self.product_url = Set(update.product_url.clone());
        self.price = Set(update.price);
        self.condition = Set(update.condition.clone());
        self.in_stock = Set(update.in_stock);
        self.updated_at = Set(chrono::Utc::now().naive_utc());
        self
    }
}

impl From<&OfferUpdate> for ActiveModel {
    fn from(update: &OfferUpdate) -> Self {
        // bookmeter_id / site は NotSet のまま返すので、呼び出し側でセットする
        <ActiveModel as Default>::default().apply_update(update)
    }
}
