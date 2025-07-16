use crate::Arbitrage;
use crate::exchange::binance::BinanceExchange;
use crate::worker::commands::StartMonitor;
use crate::worker::state::GlobalState;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use rust_decimal::{Decimal, dec};
use serde_json::json;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

pub struct Reenter {
  pub exchange: Rc<BinanceExchange>,
  pub symbol: String,
  pub arbitrage: Arc<Arbitrage>,
}

async fn spawn_live_calc(
  exchange: Rc<BinanceExchange>,
  symbol: String,
  arbitrage: Arc<Arbitrage>,
  state: Arc<GlobalState>,
) -> Reenter {
  let arbitrage_cl = Arc::clone(&arbitrage);
  let snapshot = unsafe { &mut *arbitrage_cl.snapshot.get() };

  if let Ok(order_book) = exchange.watch_orderbook(symbol.clone()).await {
    let zero = dec!(0);
    let (spot_ask, spot_bid, future_ask, future_bid) = {
      let book = order_book.borrow();
      let get_price = |side: &BTreeMap<Decimal, Decimal>| {
        side
          .iter()
          .next()
          .map(|(p, _)| p.clone())
          .unwrap_or(zero.clone())
      };

      if symbol.contains(':') {
        (
          snapshot.spot_ask.clone(),
          snapshot.spot_bid.clone(),
          get_price(&book.asks),
          get_price(&book.bids),
        )
      } else {
        (
          get_price(&book.asks),
          get_price(&book.bids),
          snapshot.future_ask.clone(),
          snapshot.future_bid.clone(),
        )
      }
    };

    let new_entry_percent = if spot_ask != zero {
      ((future_bid - spot_ask) / spot_ask) * dec!(100)
    } else {
      dec!(0)
    };

    let new_exit_percent = if future_ask != zero {
      ((spot_bid - future_ask) / future_ask) * dec!(100)
    } else {
      dec!(0)
    };

    let two_percent = dec!(2);

    let entry_delta = (new_entry_percent - snapshot.entry_percent).abs();
    let exit_delta = (new_exit_percent - snapshot.exit_percent).abs();

    let mut need_notification = entry_delta > two_percent || exit_delta > two_percent;

    snapshot.entry_percent = new_entry_percent;
    snapshot.exit_percent = new_exit_percent;

    snapshot.spot_ask = spot_ask;
    snapshot.spot_bid = spot_bid;
    snapshot.future_ask = future_ask;
    snapshot.future_bid = future_bid;

    need_notification = need_notification && 
      snapshot.spot_ask > zero && snapshot.spot_bid > zero &&
      snapshot.future_ask > zero && snapshot.future_bid > zero;

    if need_notification {
      let notification = serde_json::to_string(snapshot);

      if let Ok(json) = notification {
        let _ = state.ws_tx.try_broadcast(json);
      }
    }
  }

  Reenter {
    exchange,
    symbol,
    arbitrage: arbitrage_cl,
  }
}

pub async fn start_monitor(
  sm: StartMonitor,
  binance: Rc<BinanceExchange>,
  state: Arc<GlobalState>,
) {
  let mut tasks = FuturesUnordered::new();

  for item in sm.items.iter() {
    let spot_symbol = format!("{}/{}", item.spot.base, item.spot.quote);
    let future_symbol = format!(
      "{}/{}:{}",
      item.future.base, item.future.quote, item.future.quote
    );

    tasks.push(spawn_live_calc(
      binance.clone(),
      spot_symbol, //
      item.clone(),
      state.clone(),
    ));

    tasks.push(spawn_live_calc(
      binance.clone(),
      future_symbol,
      item.clone(),
      state.clone(),
    ));
  }

  while let Some(reenter) = tasks.next().await {
    tasks.push(spawn_live_calc(
      reenter.exchange,
      reenter.symbol,
      reenter.arbitrage.clone(),
      state.clone(),
    ));
  }
}
