use crate::base::exchange::Exchange;
use crate::utils::exchange::get_price;
use crate::worker::commands::StartMonitor;
use crate::worker::state::GlobalState;
use crate::{Arbitrage, ArbitrageSnaphot};
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use rust_decimal::dec;
use std::cmp::Reverse;
use std::sync::Arc;

pub struct Reenter<'a> {
  pub spot: &'a dyn Exchange,
  pub future: &'a dyn Exchange,
  pub symbol: String,
  pub arbitrage: Arc<Arbitrage>,
}

async fn detect_arbitrage<'a>(
  spot: &'a dyn Exchange,
  future: &'a dyn Exchange,
  symbol: String,
  arbitrage: Arc<Arbitrage>,
  state: Arc<GlobalState>,
) -> Reenter<'a> {
  let target;

  if symbol.contains(':') {
    target = future;
  } else {
    target = spot;
  }

  let result = target.watch_orderbook(symbol.clone()).await;

  let mut new_snapshot = ArbitrageSnaphot::default();

  if let Ok(order_book) = result {
    let old_snapshot = arbitrage.snapshot.load();

    let zero = dec!(0);
    let (spot_ask, spot_bid, future_ask, future_bid) = {
      let book = order_book.borrow();
      if symbol.contains(':') {
        (
          old_snapshot.spot_ask,
          old_snapshot.spot_bid,
          *get_price(&book.asks, &zero),
          get_price(&book.bids, &Reverse(zero)).0,
        )
      } else {
        (
          *get_price(&book.asks, &zero),
          get_price(&book.bids, &Reverse(zero)).0,
          old_snapshot.future_ask,
          old_snapshot.future_bid,
        )
      }
    };

    let new_entry_percent = if spot_ask != zero {
      ((future_bid - spot_ask) / spot_ask) * dec!(100)
    } else {
      dec!(0)
    }
    .trunc_with_scale(2);

    let new_exit_percent = if future_ask != zero {
      ((spot_bid - future_ask) / future_ask) * dec!(100)
    } else {
      dec!(0)
    }
    .trunc_with_scale(2);

    let percent = dec!(0.05);

    let entry_delta = (new_entry_percent - old_snapshot.entry_percent).abs();
    let exit_delta = (new_exit_percent - old_snapshot.exit_percent).abs();

    let mut need_notification = entry_delta > percent || exit_delta > percent;

    new_snapshot.entry_percent = new_entry_percent;
    new_snapshot.exit_percent = new_exit_percent;

    new_snapshot.spot_ask = spot_ask;
    new_snapshot.spot_bid = spot_bid;
    new_snapshot.future_ask = future_ask;
    new_snapshot.future_bid = future_bid;

    let not_first = new_snapshot.spot_ask > zero
      && new_snapshot.spot_bid > zero
      && new_snapshot.future_ask > zero
      && new_snapshot.future_bid > zero;

    need_notification = need_notification && not_first;

    let max = dec!(50);

    let valid = new_snapshot.entry_percent.abs() < max && new_snapshot.exit_percent.abs() < max;

    need_notification = need_notification && valid;

    if need_notification {
      let notification = serde_json::to_string(arbitrage.as_ref());

      if let Ok(json) = notification {
        let _ = state.ws_tx.try_broadcast(json);
      }
    }
  }

  arbitrage.snapshot.store(Arc::new(new_snapshot));

  Reenter {
    spot,
    future,
    symbol,
    arbitrage,
  }
}

pub async fn start_monitor(
  sm: StartMonitor,
  exchanges: &Vec<Box<dyn Exchange>>,
  state: Arc<GlobalState>,
) {
  let mut tasks = FuturesUnordered::new();

  for item in sm.items.iter() {
    let spot_symbol = format!("{}/{}", item.spot.base, item.spot.quote);
    let future_symbol = format!("{}/{}:USDT", item.future.base, item.future.quote);

    let spot = exchanges
      .iter()
      .find(|ex| ex.name().eq(&item.spot.exchange))
      .unwrap()
      .as_ref();

    let future = exchanges
      .iter()
      .find(|ex| ex.name().eq(&item.future.exchange))
      .unwrap()
      .as_ref();

    let symbols = vec![spot_symbol, future_symbol];

    for symbol in symbols {
      tasks.push(detect_arbitrage(
        spot,
        future,
        symbol, //
        item.clone(),
        state.clone(),
      ));
    }
  }

  while let Some(reenter) = tasks.next().await {
    tasks.push(detect_arbitrage(
      reenter.spot,
      reenter.future,
      reenter.symbol,
      reenter.arbitrage,
      state.clone(),
    ));
  }

  println!("Bug");
}
