use std::{cmp::Reverse, sync::Arc};

use arc_swap::ArcSwap;
use rust_decimal::{Decimal, dec};
use serde::Deserialize;
use serde_json::Value;

use crate::{
  Arbitrage, ArbitrageSnaphot,
  base::exchange::{
    assets::{Asset, MarketType},
    order::{OrderBook, OrderBookUpdate},
  },
  utils::exchange::get_price,
};

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

pub struct Result {
  pub symbol: String,
  pub arbitrage: Arc<Arbitrage>,
}

fn detect_arbitrage(
  symbol: String,
  arbitrage: Arc<Arbitrage>,
  order_book: &mut OrderBook,
) -> Result {
  let old_snapshot = arbitrage.snapshot.load();

  let zero = Decimal::ZERO;
  let (spot_ask, spot_bid, future_ask, future_bid) = {
    if symbol.contains(':') {
      (
        old_snapshot.spot_ask,
        old_snapshot.spot_bid,
        *get_price(&order_book.asks, &zero),
        get_price(&order_book.bids, &Reverse(zero)).0,
      )
    } else {
      (
        *get_price(&order_book.asks, &zero),
        get_price(&order_book.bids, &Reverse(zero)).0,
        old_snapshot.future_ask,
        old_snapshot.future_bid,
      )
    }
  };

  let new_entry_percent = if spot_ask != zero {
    ((future_bid - spot_ask) / spot_ask) * dec!(100)
  } else {
    zero
  }
  .trunc_with_scale(2);

  let new_exit_percent = if future_ask != zero {
    ((spot_bid - future_ask) / future_ask) * dec!(100)
  } else {
    zero
  }
  .trunc_with_scale(2);

  let mut new_snapshot = ArbitrageSnaphot::default();

  new_snapshot.entry_percent = new_entry_percent;
  new_snapshot.exit_percent = new_exit_percent;

  new_snapshot.spot_ask = spot_ask;
  new_snapshot.spot_bid = spot_bid;
  new_snapshot.future_ask = future_ask;
  new_snapshot.future_bid = future_bid;

  arbitrage.snapshot.store(Arc::new(new_snapshot));

  Result { symbol, arbitrage }
}

pub fn test() -> Option<()> {
  let json = "
    {\"channel\":\"futures.order_book_update\",\"event\":\"update\",\"result\":{\"t\":1753334753764,\"U\":212927173,\"u\":212927202,\"s\":\"EPIC_USDT\",\"a\":[{\"p\":\"1\",\"s\":1}],\"b\":[{\"p\":\"2\",\"s\":1}],\"l\":\"5\"},\"time\":1753334753,\"time_ms\":1753334753767}  
  ";

  let parsed: Value = serde_json::from_str(json).unwrap();
  let parsed = &parsed["result"];
  let full = parsed["full"].as_bool().unwrap_or(false);

  let last_update_id = parsed["u"].as_u64()?;
  let first_update_id = parsed["U"].as_u64()?;

  let mut book = OrderBook::default();

  let build_update = || -> Option<OrderBookUpdate> {
    let parse_side = |side: &Value| {
      side
        .as_array()?
        .iter()
        .map(|entry| {
          
          
          let tmp = FutureDepthSnapshotItem::deserialize(entry).ok()?;
          let price = tmp.p;
          let qty = tmp.s;
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

  book.apply_update(&build_update()?);

  let arbitrage = Arc::new(Arbitrage {
    spot: Asset {
      base: "EPIC".to_string(),
      symbol: "EPICUSDT".to_string(),
      quote: "USDT".to_string(),
      exchange: "Gate".to_string(),
      market: MarketType::Spot,
    },
    future: Asset {
      base: "EPIC".to_string(),
      symbol: "EPICUSDT".to_string(),
      quote: "USDT".to_string(),
      exchange: "Gate".to_string(),
      market: MarketType::Future,
    },
    snapshot: ArcSwap::new(Arc::new(ArbitrageSnaphot::default())),
  });

  let result = detect_arbitrage("EPIC/USDT:USDT".to_string(), arbitrage.clone(), &mut book);

  println!("Orderbook: {:?}", result.arbitrage.snapshot);

  Some(())
}
