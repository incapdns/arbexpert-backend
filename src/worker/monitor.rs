use crate::Arbitrage;
use crate::base::exchange::Exchange;
use crate::worker::commands::StartMonitor;
use crate::worker::state::GlobalState;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use rust_decimal::{Decimal, dec};
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct Reenter<'a> {
  pub spot: &'a dyn Exchange,
  pub future: &'a dyn Exchange,
  pub symbol: String,
  pub arbitrage: Arc<Arbitrage>,
}

async fn spawn_live_calc<'a>(
  spot: &'a dyn Exchange,
  future: &'a dyn Exchange,
  symbol: String,
  arbitrage: Arc<Arbitrage>,
  state: Arc<GlobalState>,
) -> Reenter<'a> {
  let arbitrage_cl = Arc::clone(&arbitrage);
  let snapshot = unsafe { &mut *arbitrage_cl.snapshot.get() };

  let target;

  if symbol.contains(':') {
    target = future;
  } else {
    target = spot;
  }

  if let Ok(order_book) = target.watch_orderbook(symbol.clone()).await {
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
    }
    .trunc_with_scale(2);

    let new_exit_percent = if future_ask != zero {
      ((spot_bid - future_ask) / future_ask) * dec!(100)
    } else {
      dec!(0)
    }
    .trunc_with_scale(2);

    let percent = dec!(0.05);

    let entry_delta = (new_entry_percent - snapshot.entry_percent).abs();
    let exit_delta = (new_exit_percent - snapshot.exit_percent).abs();

    let mut need_notification = entry_delta > percent || exit_delta > percent;

    snapshot.entry_percent = new_entry_percent;
    snapshot.exit_percent = new_exit_percent;

    snapshot.spot_ask = spot_ask;
    snapshot.spot_bid = spot_bid;
    snapshot.future_ask = future_ask;
    snapshot.future_bid = future_bid;

    let not_first = snapshot.spot_ask > zero
      && snapshot.spot_bid > zero
      && snapshot.future_ask > zero
      && snapshot.future_bid > zero;

    need_notification = need_notification && not_first;

    if need_notification {
      let notification = serde_json::to_string(snapshot);

      if let Ok(json) = notification {
        let _ = state.ws_tx.try_broadcast(json);
      }
    }
  }

  Reenter {
    spot,
    future,
    symbol,
    arbitrage: arbitrage_cl,
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
    let future_symbol = format!(
      "{}/{}:{}",
      item.future.base, item.future.quote, item.future.quote
    );

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
      tasks.push(spawn_live_calc(
        spot,
        future,
        symbol, //
        item.clone(),
        state.clone(),
      ));
    }
  }

  while let Some(reenter) = tasks.next().await {
    tasks.push(spawn_live_calc(
      reenter.spot,
      reenter.future,
      reenter.symbol,
      reenter.arbitrage.clone(),
      state.clone(),
    ));
  }
}
