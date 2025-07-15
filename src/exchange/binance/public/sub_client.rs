use crate::base::exchange::assets::MarketType;
use crate::base::exchange::order::OrderBook;
use crate::base::exchange::order::OrderBookUpdate;
use crate::base::exchange::sub_client::Shared;
use crate::base::exchange::sub_client::SubClient;
use crate::base::http::generic::DynamicIterator;
use crate::exchange::binance::BinanceExchangeUtils;
use crate::from_headers;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::cell::RefCell;
use std::cmp::max;
use std::collections::HashMap;
use std::error::Error;
use std::mem;
use std::rc::Rc;
use std::vec;

type Init = Rc<RefCell<HashMap<String, Vec<OrderBookUpdate>>>>;

pub struct BinanceSubClient {
  base: SubClient,
  init: Init,
}

#[derive(Debug, Deserialize)]
struct DepthSnapshot {
  #[serde(rename = "lastUpdateId")]
  last_update_id: u64,
  bids: Vec<(Decimal, Decimal)>,
  asks: Vec<(Decimal, Decimal)>,
}

impl BinanceSubClient {
  async fn process_binance_depth(
    symbol: String,
    utils: Rc<BinanceExchangeUtils>,
    initial_event_u: u64, // U do primeiro evento recebido
    market: MarketType,
  ) -> Result<OrderBook, Box<dyn std::error::Error>> {
    let mut snapshot = DepthSnapshot {
      last_update_id: 0,
      bids: vec![],
      asks: vec![],
    };

    while snapshot.last_update_id < initial_event_u {
      let uri = if let MarketType::Spot = market {
        format!(
          "https://api.binance.com/api/v3/depth?symbol={}&limit=100",
          symbol
        )
      } else {
        format!(
          "https://fapi.binance.com/fapi/v1/depth?symbol={}&limit=100",
          symbol
        )
      };

      let dummy_headers: Vec<(&str, &str)> = vec![("Accept", "application/json")];
      let headers = from_headers!(dummy_headers);

      let response = utils
        .http_client
        .request("GET".to_string(), uri, headers, None)
        .await?
        .body()
        .limit(10 * 1024 * 1024)
        .await?;

      let response = std::str::from_utf8(&response)?;

      snapshot = serde_json::from_str(response)?;
    }

    Ok(OrderBook {
      asks: snapshot.asks.into_iter().collect(),
      bids: snapshot.bids.into_iter().collect(),
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
    let mut borrow = shared.subscribed.borrow_mut();
    let book = borrow.get_mut(symbol)?;

    let last_update_id = parsed["u"].as_u64()?;
    let first_update_id = parsed["U"].as_u64()?;

    let get_update = || -> Option<OrderBookUpdate> {
      let asks = parsed["a"]
        .as_array()?
        .iter()
        .map(|item| {
          let price = item[0].as_str()?.parse::<Decimal>().ok()?;
          let qty = item[1].as_str()?.parse::<Decimal>().ok()?;
          Some((price, qty))
        })
        .collect::<Option<Vec<(Decimal, Decimal)>>>()?;

      let bids = parsed["b"]
        .as_array()?
        .iter()
        .map(|item| {
          let price = item[0].as_str()?.parse::<Decimal>().ok()?;
          let qty = item[1].as_str()?.parse::<Decimal>().ok()?;
          Some((price, qty))
        })
        .collect::<Option<Vec<(Decimal, Decimal)>>>()?;

      Some(OrderBookUpdate {
        asks,
        bids,
        update_id: last_update_id,
      })
    };

    let pass_update = |book: &mut OrderBook, update: OrderBookUpdate| -> Option<()> {
      book.apply_update(update);

      let subscription_book = Rc::new(book.clone());

      let subscriptions = mem::take(shared.pending.borrow_mut().get_mut(symbol)?);
      for subscription in subscriptions {
        let _ = subscription.send(subscription_book.clone());
      }

      Some(())
    };

    if book.update_id == 0 {
      book.update_id = 1;

      {
        let mut borrow = init.borrow_mut();
        borrow.insert(symbol.to_string(), vec![]);
      }

      drop(borrow);

      let mut processed_book = Self::process_binance_depth(
        symbol.to_string(),
        utils.clone(),
        first_update_id,
        market.clone(),
      )
      .await
      .ok()?;

      let mut borrow = init.borrow_mut();
      let mut itens = mem::take(borrow.get_mut(symbol)?);

      itens.sort_by(|a, b| a.update_id.cmp(&b.update_id));

      let future_last_id = itens
        .last()
        .map(|item| max(item.update_id, last_update_id))
        .unwrap_or(last_update_id);

      for item in itens {
        if item.update_id <= processed_book.update_id {
          continue;
        }

        processed_book.apply_update(item);
      }

      {
        let mut borrow = shared.subscribed.borrow_mut();
        let book = borrow.get_mut(symbol)?;
        book.asks = processed_book.asks;
        book.bids = processed_book.bids;
        if let MarketType::Future = market {
          book.update_id = future_last_id;
        } else {
          book.update_id = processed_book.update_id;
        }
      }
    } else if book.update_id == 1 {
      let mut borrow = init.borrow_mut();
      let itens = borrow.get_mut(symbol)?;
      itens.push(get_update()?);
    } else if let MarketType::Spot = market {
      if last_update_id <= book.update_id {
        return None;
      }

      if first_update_id > book.update_id + 1 {
        book.bids.clear();
        book.asks.clear();
        book.update_id = 0;
        return None;
      }

      pass_update(book, get_update()?);
    } else {
      let previous_last_update_id = parsed["pu"].as_u64()?;

      if previous_last_update_id != book.update_id {
        book.bids.clear();
        book.asks.clear();
        book.update_id = 0;
        return None;
      }

      pass_update(book, get_update()?);
    }

    Some(())
  }

  pub fn on_fail(init: Init) {
    init.borrow_mut().clear();
  }

  /// Cria e conecta imediatamente
  pub fn new(utils: Rc<BinanceExchangeUtils>, market: MarketType) -> Self {
    let ws_url = if let MarketType::Spot = market {
      "wss://stream.binance.com/ws"
    } else {
      "wss://fstream.binance.com/ws"
    };

    let init = Rc::new(RefCell::new(HashMap::new()));

    let (ic1, ic2) = (init.clone(), init.clone());

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
      ),
      init,
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

  pub async fn watch(&mut self, symbol: &str) -> Result<Rc<OrderBook>, Box<dyn Error>> {
    self.base.watch(symbol).await
  }

  pub async fn unwatch(&mut self, symbol: &str) -> Result<(), Box<dyn Error>> {
    self.base.unwatch(symbol).await
  }
}
