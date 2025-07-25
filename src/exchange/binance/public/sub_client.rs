use crate::base::exchange::assets::MarketType;
use crate::base::exchange::order::OrderBook;
use crate::base::exchange::order::OrderBookUpdate;
use crate::base::exchange::sub_client::Shared;
use crate::base::exchange::sub_client::SharedBook;
use crate::base::exchange::sub_client::SubClient;
use crate::base::http::generic::DynamicIterator;
use crate::exchange::binance::BinanceExchangeUtils;
use crate::from_headers;
use once_cell::sync::OnceCell;
use ratelimit::Alignment;
use ratelimit::Ratelimiter;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::error::Error;
use std::mem;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use std::usize;

type Init = Rc<RefCell<HashMap<String, Vec<OrderBookUpdate>>>>;

pub struct BinanceSubClient {
  base: SubClient,
}

#[derive(Debug, Deserialize)]
struct DepthSnapshot {
  #[serde(rename = "lastUpdateId")]
  last_update_id: u64,
  bids: Vec<(Decimal, Decimal)>,
  asks: Vec<(Decimal, Decimal)>,
}

pub static CONNECT_LIMITER: OnceCell<Arc<Ratelimiter>> = OnceCell::new();

pub static SPOT_HTTP_LIMITER: OnceCell<Arc<Ratelimiter>> = OnceCell::new();

pub static FUTURE_HTTP_LIMITER: OnceCell<Arc<Ratelimiter>> = OnceCell::new();

impl BinanceSubClient {
  async fn process_binance_depth(
    symbol: &str,
    utils: Rc<BinanceExchangeUtils>,
    initial_event_u: u64,
    market: MarketType,
  ) -> Result<OrderBook, Box<dyn std::error::Error>> {
    let mut snapshot = DepthSnapshot {
      last_update_id: 0,
      bids: Vec::new(),
      asks: Vec::new(),
    };

    while snapshot.last_update_id < initial_event_u {
      let limiter;
      let uri = if let MarketType::Spot = market {
        limiter = &SPOT_HTTP_LIMITER;
        format!("https://api.binance.com/api/v3/depth?symbol={symbol}&limit=100")
      } else {
        limiter = &FUTURE_HTTP_LIMITER;
        format!("https://fapi.binance.com/fapi/v1/depth?symbol={symbol}&limit=100")
      };

      let limiter = limiter.get().expect("Limiter not initialized");

      let headers = from_headers!([("Accept", "application/json")]);

      let response = loop {
        match limiter.try_wait_n(5) {
          Ok(()) => {
            break utils
              .http_client
              .request("GET".into(), uri, headers, None)
              .await?
              .body()
              .limit(10 * 1024 * 1024)
              .await?;
          }
          Err(duration) => {
            ntex::time::sleep(duration).await;
          }
        }
      };

      snapshot = serde_json::from_slice(&response)?; // Evita utf8 + alloc
    }

    Ok(OrderBook {
      asks: snapshot.asks.into_iter().collect(), // Decimal -> Decimal, OK
      bids: snapshot
        .bids
        .into_iter()
        .map(|(price, qty)| (Reverse(price), qty)) // <- Aqui converte
        .collect(),
      update_id: snapshot.last_update_id,
    })
  }

  pub async fn handle_message(
    text: &str,
    shared: Shared,
    init: Init,
    utils: Rc<BinanceExchangeUtils>,
    market: MarketType,
  ) -> Option<()> {
    let parsed: Value = serde_json::from_str(text).ok()?;
    let symbol = parsed["s"].as_str()?;
    let last_update_id = parsed["u"].as_u64()?;
    let first_update_id = parsed["U"].as_u64()?;

    let book = {
      let borrow = shared.subscribed.borrow();
      borrow.get(symbol)?.clone()
    };

    let update_id = { book.borrow().update_id };

    let build_update = || -> Option<OrderBookUpdate> {
      let parse_side = |side: &Value| {
        side
          .as_array()?
          .iter()
          .map(|entry| {
            let price = entry[0].as_str()?.parse::<Decimal>().ok()?;
            let qty = entry[1].as_str()?.parse::<Decimal>().ok()?;
            Some((price, qty))
          })
          .collect::<Option<Vec<_>>>()
      };

      Some(OrderBookUpdate {
        asks: parse_side(&parsed["a"])?,
        bids: parse_side(&parsed["b"])?,
        last_update_id,
        first_update_id,
        full: false,
      })
    };

    let broadcast_update = |book: SharedBook, update: OrderBookUpdate| -> Option<()> {
      book.borrow_mut().apply_update(&update, &mut vec![]);
      let subscriptions = mem::take(shared.pending.borrow_mut().get_mut(symbol)?);
      for sub in subscriptions {
        let _ = sub.send(book.clone());
      }

      Some(())
    };

    if update_id == 0 {
      book.borrow_mut().update_id = 1;
      init.borrow_mut().entry(symbol.to_string()).or_default();

      let snapshot =
        Self::process_binance_depth(symbol, utils.clone(), first_update_id, market.clone()).await;

      if snapshot.is_err() {
        book.borrow_mut().update_id = 0;
        return Some(());
      }

      let snapshot = snapshot.ok()?;

      let mut retries = 0;
      let processed = loop {
        if retries == 5 {
          book.borrow_mut().update_id = 0;
          return Some(());
        }

        let mut pending = mem::take(init.borrow_mut().get_mut(symbol)?);

        let idx = pending.iter().position(|item| {
          item.first_update_id <= snapshot.update_id + 1
            && item.last_update_id > snapshot.update_id
        });

        if idx.is_none() {
          ntex::time::sleep(Duration::from_millis(100)).await;
          retries += 1;
          continue;
        }

        pending.drain(0..idx.unwrap());

        break pending;
      };

      let mut book_bm = book.borrow_mut();
      book_bm.asks = snapshot.asks;
      book_bm.bids = snapshot.bids;
      for update in processed {
        book_bm.apply_update(&update, &mut vec![]);
      }
    } else if update_id == 1 {
      init.borrow_mut().get_mut(symbol)?.push(build_update()?);
    } else if let MarketType::Spot = market {
      {
        let mut book_mut = book.borrow_mut();

        if first_update_id > book_mut.update_id + 1 || last_update_id < book_mut.update_id + 1 {
          book_mut.asks.clear();
          book_mut.bids.clear();
          book_mut.update_id = 0;
          return Some(());
        }
      }

      broadcast_update(book.clone(), build_update()?);
    } else {
      {
        let previous_id = parsed["pu"].as_u64()?;
        let mut book_mut = book.borrow_mut();

        if previous_id != book_mut.update_id {
          book_mut.asks.clear();
          book_mut.bids.clear();
          book_mut.update_id = 0;
          return Some(());
        }
      }

      broadcast_update(book.clone(), build_update()?);
    }

    Some(())
  }

  pub fn on_fail(init: Init) {
    init.borrow_mut().clear();
  }

  /// Cria e conecta imediatamente
  pub fn new(utils: Rc<BinanceExchangeUtils>, time_offset_ms: i64, market: MarketType) -> Self {
    let ws_url = if let MarketType::Spot = market {
      "wss://stream.binance.com/ws"
    } else {
      "wss://fstream.binance.com/ws"
    };

    let init = Rc::new(RefCell::new(HashMap::new()));

    let (ic1, ic2) = (init.clone(), init);

    let now = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("Time went backwards")
      .as_millis() as i64;

    let server_time = now + time_offset_ms;

    let send_limiter = Arc::new(
      Ratelimiter::builder(4, Duration::from_secs(5))
        .max_tokens(4)
        .initial_available(4)
        .alignment(Alignment::Second)
        .sync_time(server_time as u64)
        .build()
        .unwrap(),
    );

    let (http_limiter, http_weight) = if market.eq(&MarketType::Future) {
      (&FUTURE_HTTP_LIMITER, 5)
    } else {
      (&SPOT_HTTP_LIMITER, 2)
    };

    let http_limiter = http_limiter
      .get()
      .cloned()
      .expect("Limiter not initialized");    

    BinanceSubClient {
      base: SubClient::new(
        ws_url,
        move |text, shared| {
          let ic1 = ic1.clone();
          let utils = utils.clone();
          let market = market.clone();
          async move {
            Self::handle_message(&text, shared, ic1, utils, market).await;
          }
        },
        move |_, _| async {},
        move || {
          Self::on_fail(ic2.clone());
        },
        Self::subscribe,
        Self::unsubscribe,
        CONNECT_LIMITER
          .get()
          .cloned()
          .expect("Limiter not initialized"),
        send_limiter,
        http_limiter,
        http_weight
      ),
    }
  }

  /// Diz se este subcliente já tem `symbol`
  pub fn has_symbol(&self, symbol: &str) -> bool {
    self.base.has_symbol(symbol)
  }

  /// Quantos canais já foram inscritos
  pub fn subscribed_count(&self) -> usize {
    self.base.subscribed_count()
  }

  /// Pega o _próximo_ OrderBook para `symbol`. Se for a primeira chamada, envia SUBSCRIBE.
  pub async fn subscribe(symbol: String) -> Result<String, Box<dyn Error>> {
    let msg = json!({
      "method": "SUBSCRIBE",
      "params": [
        format!("{}@depth@100ms", symbol.to_lowercase()),
      ],
    })
    .to_string();

    Ok(msg)
  }

  /// Cancela inscrição e limpa pendentes
  pub async fn unsubscribe(symbol: String) -> Result<String, Box<dyn Error>> {
    let msg = json!({
      "method": "UNSUBSCRIBE",
      "params": [
        format!("{}@depth@100ms", symbol.to_lowercase()),
      ],
    })
    .to_string();

    Ok(msg)
  }

  pub async fn watch(&self, symbol: &str) -> Result<SharedBook, Box<dyn Error>> {
    self.base.watch(symbol).await
  }

  pub async fn unwatch(&self, symbol: &str) -> Result<(), Box<dyn Error>> {
    self.base.unwatch(symbol).await
  }
}
