use serde::{Deserialize, Serialize};
use std::{collections::{BTreeMap, HashMap}, sync::{LazyLock, Mutex}};

use crate::{base::http::generic::DynamicIterator, exchange::mexc::MexcExchange, from_headers};

#[derive(Debug, Deserialize)]
pub struct ApiResponse {
  pub data: SymbolData,
}

#[derive(Debug, Deserialize)]
pub struct SymbolData {
  pub symbols: HashMap<String, Vec<SymbolItem>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SymbolItem {
  pub cd: String,  // item ID
  pub vn: String,  // item name
  pub mcd: String, // symbol ID
  #[serde(alias = "in")]
  pub image: String, // symbol ID
}

#[derive(Clone, Debug, Serialize)]
pub struct Item {
  pub id: String,
  pub image: String
}

#[derive(Clone, Debug, Serialize)]
pub struct SymbolEntry {
  pub id: String,
  pub items: BTreeMap<String, Item>,
}

pub type Symbols = HashMap<String, SymbolEntry>;

static SYMBOLS: LazyLock<Mutex<Symbols>> = LazyLock::new(|| Mutex::new(Symbols::new()));

impl MexcExchange {
  pub async fn fetch_symbols(&self) -> Result<Symbols, Box<dyn std::error::Error>> {
    let mut lock = SYMBOLS.lock().unwrap();
    if !lock.is_empty() {
      return Ok(lock.clone())
    }

    let client = &self
      .utils
      .http_client;

    let mut headers: HashMap<String, String> = HashMap::new();
    headers.insert("Host".to_string(), "www.mexc.com".to_string());
    headers.insert("User-Agent".to_string(), "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36".to_string());
    headers.insert("Accept".to_string(), "*/*".to_string());
    headers.insert("Accept-Encoding".to_string(), "gzip, deflate".to_string());

    let response = client
      .request(
        "GET".into(),
        "https://www.mexc.com/api/platform/spot/market-v2/web/symbolsV2".into(),
        from_headers!(headers), // no headers
        None,
      )
      .await;

    let body = response?
      .body()
      .limit(10 * 1024 * 1024)
      .await
      .map_err(|e| format!("Failed to read body: {e}"))?;

    let parsed: ApiResponse = serde_json::from_slice(&body)?;

    let mut symbols: Symbols = HashMap::new();

    for (name, items) in parsed.data.symbols {
      let mut item_map = BTreeMap::new();

      for item in items.iter() {
        item_map.insert(
          item.vn.clone(),
          Item {
            id: item.cd.clone(),
            image: item.image.clone()
          },
        );
      }

      symbols.insert(
        name,
        SymbolEntry {
          id: items[0].mcd.clone(),
          items: item_map,
        },
      );
    }

    *lock = symbols.clone();

    Ok(symbols)
  }

  pub fn get_symbols(&self) -> HashMap<String, SymbolEntry> {
    self.public.symbols.clone()
  }
}
