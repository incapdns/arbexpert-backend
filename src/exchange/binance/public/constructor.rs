use std::{
  cell::RefCell,
  collections::HashMap,
  ops::DerefMut,
  rc::Rc,
  sync::{Arc, LazyLock, Mutex},
  time::{Duration, SystemTime, UNIX_EPOCH},
};

use futures::join;
use ratelimit::{Alignment, Ratelimiter};

use crate::{
  base::{
    exchange::{
      assets::{Asset, Assets, MarketType},
      error::ExchangeError,
    },
    http::{client::ntex::NtexHttpClient, generic::DynamicIterator},
  },
  exchange::binance::{
    BinanceExchange, BinanceExchangePublic, BinanceExchangeUtils,
    public::sub_client::{CONNECT_LIMITER, FUTURE_HTTP_LIMITER, SPOT_HTTP_LIMITER},
  },
  from_headers,
};

static TIME_OFFSET_MS: LazyLock<Mutex<i64>> = LazyLock::new(|| Mutex::new(i64::MAX));
static ASSETS: LazyLock<Mutex<Assets>> = LazyLock::new(|| Mutex::new(Assets::new()));

static LOAD_ASSETS: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static SYNC_TIME: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

impl BinanceExchange {
  pub async fn new() -> Self {
    let mut result = Self {
      public: BinanceExchangePublic {
        spot_clients: RefCell::new(Vec::new()),
        future_clients: RefCell::new(Vec::new()),
        assets: None,
        time_offset_ms: 0,
      },
      utils: Rc::new(BinanceExchangeUtils::new(NtexHttpClient::new())),
    };

    let _ = result.sync_time().await;
    let _ = result.load_assets().await;

    result
  }

  pub fn name(&self) -> String {
    "Binance".to_string()
  }

  #[allow(static_mut_refs)]
  async fn fetch_assets_by(&self, market: MarketType) -> Result<Vec<Asset>, ExchangeError> {
    let headers: HashMap<String, String> = HashMap::new();

    let limiter;
    let url = if let MarketType::Spot = market {
      limiter = (20, unsafe { &SPOT_HTTP_LIMITER });
      "https://api.binance.com/api/v3/exchangeInfo"
    } else {
      limiter = (1, unsafe { &FUTURE_HTTP_LIMITER });
      "https://fapi.binance.com/fapi/v1/exchangeInfo"
    };

    let response = loop {
      match limiter.1.try_wait_n(limiter.0) {
        Ok(()) => {
          break self
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
        }
        Err(duration) => {
          ntex::time::sleep(duration).await;
        }
      }
    };

    let response = std::str::from_utf8(&response)
      .map_err(|e| ExchangeError::ApiError(format!("Resposta inválida: {:?}", e)))?;

    let json: serde_json::Value =
      serde_json::from_str(response).map_err(ExchangeError::JsonError)?;

    let mut assets = Vec::new();

    if let Some(symbols) = json.get("symbols").and_then(|v| v.as_array()) {
      for symbol in symbols {
        if symbol.get("status").and_then(|v| v.as_str()) != Some("TRADING") {
          continue;
        }

        let base = symbol
          .get("baseAsset")
          .and_then(|v| v.as_str())
          .unwrap_or_default();
        let quote = symbol
          .get("quoteAsset")
          .and_then(|v| v.as_str())
          .unwrap_or_default();
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
          exchange: "Binance".to_string(),
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

    let mut_lock = lock.deref_mut();

    mut_lock.spot = spot;
    mut_lock.future = future;

    Ok(mut_lock.clone())
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

  #[allow(static_mut_refs)]
  pub async fn sync_time(&mut self) -> Result<(), ExchangeError> {
    let _lock = SYNC_TIME.lock();
    let mut lock = TIME_OFFSET_MS.lock().unwrap();
    if *lock != i64::MAX {
      self.public.time_offset_ms = *lock;
      return Ok(());
    }

    let url = "https://api.binance.com/api/v3/time";
    let headers: HashMap<String, String> = HashMap::new();

    let response = loop {
      match unsafe { &SPOT_HTTP_LIMITER }.try_wait() {
        Ok(()) => {
          break self
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
        }
        Err(duration) => {
          ntex::time::sleep(duration).await;
        }
      }
    };

    let response = std::str::from_utf8(&response)
      .map_err(|e| ExchangeError::ApiError(format!("Invalid response {:?}", e)))?;

    let json: serde_json::Value =
      serde_json::from_str(&response).map_err(ExchangeError::JsonError)?;

    if let Some(server_time) = json.get("serverTime").and_then(|v| v.as_i64()) {
      let local_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

      self.public.time_offset_ms = server_time - local_time;
      *lock = self.public.time_offset_ms;

      let connect_limiter = Ratelimiter::builder(300, Duration::from_secs(300))
        .max_tokens(300)
        .initial_available(300)
        .alignment(Alignment::Minute)
        .sync_time(server_time as u64)
        .build()
        .unwrap();

      unsafe {
        let arc_mut = &mut *CONNECT_LIMITER;
        let connect_limit_ptr = Arc::as_ptr(arc_mut) as *mut Ratelimiter;
        *connect_limit_ptr = connect_limiter;
      }

      let spot_http_limiter = Ratelimiter::builder(5950, Duration::from_millis(61500))
        .max_tokens(5950)
        .initial_available(5950)
        .alignment(Alignment::Minute)
        .sync_time(server_time as u64)
        .build()
        .unwrap();

      unsafe {
        let arc_mut = &mut *SPOT_HTTP_LIMITER;
        let spot_limit_ptr = Arc::as_ptr(arc_mut) as *mut Ratelimiter;
        *spot_limit_ptr = spot_http_limiter;
      }

      let future_http_limiter = Ratelimiter::builder(2350, Duration::from_millis(61500))
        .max_tokens(2350)
        .initial_available(2350)
        .alignment(Alignment::Minute)
        .sync_time(server_time as u64)
        .build()
        .unwrap();

      unsafe {
        let arc_mut = &mut *FUTURE_HTTP_LIMITER;
        let future_limit_ptr = Arc::as_ptr(arc_mut) as *mut Ratelimiter;
        *future_limit_ptr = future_http_limiter;
      }

      Ok(())
    } else {
      Err(ExchangeError::ApiError(
        "Invalid server time response".into(),
      ))
    }
  }
}
