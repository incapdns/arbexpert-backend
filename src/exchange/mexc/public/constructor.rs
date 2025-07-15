use std::{
  cell::RefCell,
  collections::HashMap,
  rc::Rc,
  sync::{LazyLock, Mutex},
  time::{SystemTime, UNIX_EPOCH},
};

use crate::{
  base::{
    exchange::error::ExchangeError,
    http::{client::ntex::NtexHttpClient, generic::DynamicIterator},
  },
  exchange::mexc::{private::empty_private, MexcExchange, MexcExchangePublic, MexcExchangeUtils}, from_headers,
};

static TIME_OFFSET_MS: LazyLock<Mutex<i64>> = LazyLock::new(|| Mutex::new(i64::MAX));

impl MexcExchange {
  pub async fn new() -> Self {
    let mut result = Self {
      private: empty_private(),
      public: MexcExchangePublic {
        symbols: HashMap::new(),
        sub_clients: Vec::with_capacity(10),
        markets: None,
        pairs: Rc::new(RefCell::new(HashMap::new())),
        time_offset_ms: 0,
      },
      utils: Rc::new(MexcExchangeUtils::new(NtexHttpClient::new())),
    };

    let _ = result.sync_time().await;
    result.public.symbols = result.fetch_symbols().await.unwrap_or_default();

    result
  }

  pub async fn sync_time(&mut self) -> Result<(), ExchangeError> {
    let mut lock = TIME_OFFSET_MS.lock().unwrap();
    if *lock != i64::MAX {
      self.public.time_offset_ms = *lock;
      return Ok(());
    }
    
    let url = "https://api.mexc.com/api/v3/time";
    let headers: HashMap<String, String> = HashMap::new();

    let resp = self
      .utils
      .http_client
      .request("GET".to_string(), url.to_string(), from_headers!(headers), None) // ajuste conforme seu client
      .await
      .map_err(|e| ExchangeError::ApiError(format!("Sync time error: {}", e)))?
      .body()
      .await
      .map_err(|e| ExchangeError::ApiError(format!("Invalid body: {}", e)))?;

    let resp = std::str::from_utf8(&resp)
      .map_err(|e| ExchangeError::ApiError(format!("Invalid response {:?}", e)))?;

    let json: serde_json::Value = serde_json::from_str(&resp).map_err(ExchangeError::JsonError)?;

    if let Some(server_time) = json.get("serverTime").and_then(|v| v.as_i64()) {
      let local_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

      self.public.time_offset_ms = server_time - local_time;
      *lock = self.public.time_offset_ms;
      Ok(())
    } else {
      Err(ExchangeError::ApiError(
        "Invalid server time response".into(),
      ))
    }
  }
}
