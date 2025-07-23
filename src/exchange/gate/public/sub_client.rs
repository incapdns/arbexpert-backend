use crate::base::exchange::assets::MarketType;
use crate::base::exchange::order::OrderBook;
use crate::base::exchange::order::OrderBookUpdate;
use crate::base::exchange::sub_client::Shared;
use crate::base::exchange::sub_client::SharedBook;
use crate::base::exchange::sub_client::SubClient;
use crate::base::http::generic::DynamicIterator;
use crate::exchange::gate::GateExchangeUtils;
use crate::from_headers;
use once_cell::sync::Lazy;
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

type Init = Rc<RefCell<HashMap<String, Vec<OrderBookUpdate>>>>;

pub struct GateSubClient {
  base: SubClient,
}

#[derive(Debug, Deserialize)]
struct SpotDepthSnapshot {
  id: u64,
  bids: Vec<(Decimal, Decimal)>,
  asks: Vec<(Decimal, Decimal)>,
}

#[derive(Debug, Deserialize)]
struct FutureDepthSnapshotItem {
  p: Decimal,
  s: Decimal,
}

#[derive(Debug, Deserialize)]
struct FutureDepthSnapshot {
  id: u64,
  bids: Vec<FutureDepthSnapshotItem>,
  asks: Vec<FutureDepthSnapshotItem>,
}

pub static mut CONNECT_LIMITER: Lazy<Arc<Ratelimiter>> = Lazy::new(|| {
  Arc::new(
    Ratelimiter::builder(300, Duration::from_secs(1000))
      .max_tokens(300)
      .initial_available(300)
      .build()
      .unwrap(),
  )
});

pub static mut HTTP_LIMITER: Lazy<Arc<Ratelimiter>> = Lazy::new(|| {
  Arc::new(
    Ratelimiter::builder(183, Duration::from_millis(10000))
      .max_tokens(183)
      .initial_available(183)
      .build()
      .unwrap(),
  )
});

static mut TESTING: bool = true;

impl GateSubClient {
  #[allow(static_mut_refs)]
  async fn process_gate_depth(
    symbol: &str,
    utils: Rc<GateExchangeUtils>,
    initial_event_u: u64,
    market: MarketType,
  ) -> Result<OrderBook, Box<dyn std::error::Error>> {
    let mut processed = OrderBook::default();

    let mut retries = 0;

    while processed.update_id < initial_event_u {
      if retries == 5 {
        return Err("Max retry".into());
      }

      let uri = match market {
        MarketType::Spot => {
          format!(
            "https://api.gateio.ws/api/v4/spot/order_book?currency_pair={}&limit=100&with_id=true",
            symbol
          )
        }
        MarketType::Future => {
          format!(
            "https://fx-api.gateio.ws/api/v4/futures/usdt/order_book?contract={}&limit=100&with_id=true",
            symbol
          )
        }
      };

      let headers = from_headers!([("Accept", "application/json")]);

      let response = loop {
        match unsafe { &HTTP_LIMITER }.try_wait() {
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

      if let MarketType::Future = market {
        let result = serde_json::from_slice::<FutureDepthSnapshot>(&response)?;

        processed = OrderBook {
          asks: result
            .asks
            .into_iter()
            .map(|item| (item.p, item.s))
            .collect(),
          bids: result
            .bids
            .into_iter()
            .map(|item| (Reverse(item.p), item.s))
            .collect(),
          update_id: result.id,
        };
      } else {
        let result = serde_json::from_slice::<SpotDepthSnapshot>(&response)?;

        processed = OrderBook {
          asks: result.asks.into_iter().collect(),
          bids: result
            .bids
            .into_iter()
            .map(|(k, v)| (Reverse(k), v))
            .collect(),
          update_id: result.id,
        };
      }

      retries += 1;
    }

    Ok(processed)
  }

  #[allow(static_mut_refs)]
  pub async fn handle_message(
    text: &str,
    shared: Shared,
    init: Init,
    utils: Rc<GateExchangeUtils>,
    market: MarketType,
  ) -> Option<()> {
    let parsed: Value = serde_json::from_str(text).ok()?;
    let parsed = &parsed["result"];
    let full = parsed["full"].as_bool().unwrap_or(false);
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
            let price;
            let qty;
            if let MarketType::Spot = market {
              price = entry[0].as_str()?.parse::<Decimal>().ok()?;
              qty = entry[1].as_str()?.parse::<Decimal>().ok()?;
            } else {
              let tmp = FutureDepthSnapshotItem::deserialize(entry).ok()?;
              price = tmp.p;
              qty = tmp.s;
            }
            Some((price, qty))
          })
          .collect::<Option<Vec<_>>>()
      };

      Some(OrderBookUpdate {
        asks: parse_side(&parsed["a"])?,
        bids: parse_side(&parsed["b"])?,
        last_update_id,
        first_update_id,
        full,
      })
    };

    let broadcast_update = |book: SharedBook, update: OrderBookUpdate| -> Option<()> {
      book.borrow_mut().apply_update(&update);
      let subscriptions = mem::take(shared.pending.borrow_mut().get_mut(symbol)?);
      for sub in subscriptions {
        let _ = sub.send(book.clone());
      }

      Some(())
    };

    if update_id == 0 {
      if full {
        broadcast_update(book, build_update()?);
        return Some(());
      }

      book.borrow_mut().update_id = 1;
      init.borrow_mut().entry(symbol.to_string()).or_default();

      let snapshot =
        Self::process_gate_depth(symbol, utils.clone(), first_update_id, market.clone()).await;

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
            && item.last_update_id >= snapshot.update_id + 1
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
        book_bm.apply_update(&update);
      }
    } else if update_id == 1 {
      init.borrow_mut().get_mut(symbol)?.push(build_update()?);
    } else {
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
    }

    Some(())
  }

  pub fn on_fail(init: Init) {
    init.borrow_mut().clear();
  }

  #[allow(static_mut_refs)]
  pub fn new(utils: Rc<GateExchangeUtils>, market: MarketType, time_offset_ms: i64) -> Self {
    let ws_url = if let MarketType::Spot = market {
      "wss://api.gateio.ws/ws/v4/"
    } else {
      "wss://fx-ws.gateio.ws/v4/ws/usdt"
    };

    let init = Rc::new(RefCell::new(HashMap::new()));

    let (ic1, ic2) = (init.clone(), init);

    let market_cl = market.clone();

    let (m1, m2) = (market_cl.clone(), market_cl);

    let now = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("Time went backwards")
      .as_millis() as i64;

    let server_time = now + time_offset_ms;

    let send_limiter = Arc::new(
      Ratelimiter::builder(5, Duration::from_millis(2050))
        .max_tokens(5)
        .initial_available(5)
        .alignment(Alignment::Second)
        .sync_time(server_time as u64)
        .build()
        .unwrap(),
    );

    GateSubClient {
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
        move |symbol| Self::subscribe(m1.clone(), time_offset_ms, symbol),
        move |symbol| Self::unsubscribe(m2.clone(), time_offset_ms, symbol),
        unsafe { CONNECT_LIMITER.clone() },
        send_limiter,
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
  pub async fn subscribe(
    market: MarketType,
    time_offset_ms: i64,
    symbol: String,
  ) -> Result<String, Box<dyn Error>> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as i64;

    let channel;

    if let MarketType::Spot = market {
      channel = "spot.order_book_update";
    } else {
      channel = "futures.order_book_update"
    }

    let timestamp = now + time_offset_ms;
    let msg = json!({
      "time": timestamp,
      "channel": channel,
      "event": "subscribe",
      "payload": [symbol, "100ms", "100"]
    })
    .to_string();

    Ok(msg)
  }

  /// Cancela inscrição e limpa pendentes
  pub async fn unsubscribe(
    market: MarketType,
    time_offset_ms: i64,
    symbol: String,
  ) -> Result<String, Box<dyn Error>> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as i64;

    let channel;

    if let MarketType::Spot = market {
      channel = "spot.order_book_update";
    } else {
      channel = "futures.order_book_update"
    }

    let timestamp = now + time_offset_ms;
    let msg = json!({
      "time": timestamp,
      "channel": channel,
      "event": "unsubscribe",
      "payload": [symbol, "100ms", "100"]
    })
    .to_string();

    Ok(msg)
  }

  #[allow(static_mut_refs)]
  pub async fn watch(&self, symbol: &str) -> Result<SharedBook, Box<dyn Error>> {
    loop {
      match unsafe { &HTTP_LIMITER }.try_wait() {
        Ok(()) => return self.base.watch(symbol).await,
        Err(duration) => {
          ntex::time::sleep(duration).await;
        }
      }
    }
  }

  pub async fn unwatch(&self, symbol: &str) -> Result<(), Box<dyn Error>> {
    self.base.unwatch(symbol).await
  }
}
