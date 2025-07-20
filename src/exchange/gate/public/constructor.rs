use std::{
  cell::RefCell,
  collections::HashMap,
  rc::Rc,
  sync::{LazyLock, Mutex},
  time::{SystemTime, UNIX_EPOCH},
};

use futures::join;

use crate::{
  base::{
    exchange::assets::{Asset, Assets, MarketType},
    exchange::error::ExchangeError,
    http::{client::ntex::NtexHttpClient, generic::DynamicIterator},
  },
  exchange::gate::{GateExchange, GateExchangePublic, GateExchangeUtils},
  from_headers,
};

static TIME_OFFSET_MS: LazyLock<Mutex<i64>> = LazyLock::new(|| Mutex::new(i64::MAX));
static ASSETS: LazyLock<Mutex<Assets>> = LazyLock::new(|| Mutex::new(Assets::new()));

static LOAD_ASSETS: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static SYNC_TIME: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

impl GateExchange {
  pub async fn new() -> Self {
    let mut result = Self {
      public: GateExchangePublic {
        spot_clients: RefCell::new(Vec::new()),
        future_clients: RefCell::new(Vec::new()),
        assets: None,
        pairs: Rc::new(RefCell::new(HashMap::new())),
        time_offset_ms: 0,
      },
      utils: Rc::new(GateExchangeUtils::new(NtexHttpClient::new())),
    };

    let _ = result.sync_time().await;
    let _ = result.load_assets().await;

    result
  }

  pub fn name(&self) -> String {
    "Gate".to_string()
  }

  async fn fetch_assets_by(&self, market: MarketType) -> Result<Vec<Asset>, ExchangeError> {
    let headers: HashMap<String, String> = HashMap::new();

    let url = if let MarketType::Spot = market {
      "https://api.gateio.ws/api/v4/spot/currency_pairs"
    } else {
      "https://fx-api.gateio.ws/api/v4/futures/usdt/contracts"
    };

    let resp = self
      .utils
      .http_client
      .request(
        "GET".to_string(),
        url.to_string(),
        from_headers!(headers),
        None,
      )
      .await
      .map_err(|e| ExchangeError::ApiError(format!("Erro ao buscar ativos: {}", e)))?
      .body()
      .limit(100 * 1024 * 1024)
      .await
      .map_err(|e| ExchangeError::ApiError(format!("Corpo inválido: {}", e)))?;

    let resp_str = std::str::from_utf8(&resp)
      .map_err(|e| ExchangeError::ApiError(format!("Resposta inválida: {:?}", e)))?;

    let json: serde_json::Value =
      serde_json::from_str(resp_str).map_err(ExchangeError::JsonError)?;

    let mut assets = Vec::new();

    if let Some(symbols) = json.as_array() {
      for symbol in symbols {
        if let MarketType::Future = market {
          if symbol.get("status").and_then(|v| v.as_str()) != Some("trading") {
            continue;
          }
        } else {
           if symbol.get("trade_status").and_then(|v| v.as_str()) != Some("tradable") {
            continue;
          }
        }

        let name;

        if let MarketType::Future = market {
          name = symbol
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        } else {
          name = symbol
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        }

        let mut parts = name.splitn(2, '_');

        let base = parts.next().unwrap_or("");
        let quote = parts.next().unwrap_or("");

        let symbol_name;

        if let MarketType::Spot = market {
          symbol_name = format!("{}/{}", base, quote);
        } else {
          symbol_name = format!("{}/{}:{}", base, quote, quote);
        }

        assets.push(Asset {
          symbol: symbol_name,
          base: base.to_string(),
          quote: quote.to_string(),
          market: market.clone(),
          exchange: "Gate".to_string(),
        });
      }
    }

    Ok(assets)
  }

  pub async fn fetch_assets(&mut self) -> Result<Assets, ExchangeError> {
    let (spot, future) = join!(
      self.fetch_assets_by(MarketType::Spot),
      self.fetch_assets_by(MarketType::Future)
    );

    let (spot, future) = (spot?, future?);

    let spot = spot
      .into_iter()
      .map(|asset| (asset.symbol.clone(), asset))
      .collect();

    let future = future
      .into_iter()
      .map(|asset| (asset.symbol.clone(), asset))
      .collect();

    let mut lock = ASSETS.lock().unwrap();

    lock.spot = spot;
    lock.future = future;

    Ok(lock.clone())
  }

  pub async fn load_assets(&mut self) -> Result<Assets, ExchangeError> {
    let _lock = LOAD_ASSETS.lock();
    {
      let lock = ASSETS.lock().unwrap();
      if lock.spot.len() > 0 {
        self.public.assets = Some(lock.clone());
        return Ok(lock.clone());
      }
    }

    let assets = self.fetch_assets().await?;

    self.public.assets = Some(assets);

    Ok(self.public.assets.clone().unwrap())
  }

  pub async fn sync_time(&mut self) -> Result<(), ExchangeError> {
    let _lock = SYNC_TIME.lock();
    let mut lock = TIME_OFFSET_MS.lock().unwrap();
    if *lock != i64::MAX {
      self.public.time_offset_ms = *lock;
      return Ok(());
    }

    let url = "https://api.Gate.com/api/v3/time";
    let headers: HashMap<String, String> = HashMap::new();

    let resp = self
      .utils
      .http_client
      .request(
        "GET".to_string(),
        url.to_string(),
        from_headers!(headers),
        None,
      ) // ajuste conforme seu client
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
