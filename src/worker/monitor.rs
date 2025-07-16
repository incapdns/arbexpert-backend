use crate::Arbitrage;
use crate::exchange::binance::BinanceExchange;
use crate::worker::commands::StartMonitor;
use crate::worker::state::GlobalState;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use rust_decimal::dec;
use serde_json::json;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

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
  let order_book = exchange.watch_orderbook(symbol.clone()).await;

  let arbitrage_cl = arbitrage.clone();

  let snapshot = unsafe { &mut *arbitrage_cl.as_ref().snapshot.get() };

  if let Ok(ob) = order_book {
    let zero = dec!(0);
    let ob = ob.borrow();
    
    if symbol.contains(':') {
      let (price, _) = ob.asks.iter().next().unwrap_or((&zero, &zero));
      snapshot.future_ask = price.clone();

      let (price, _) = ob.bids.iter().next().unwrap_or((&zero, &zero));
      snapshot.future_bid = price.clone();
    } else {
      let (price, _) = ob.asks.iter().next().unwrap_or((&zero, &zero));
      snapshot.spot_ask = price.clone();

      let (price, _) = ob.bids.iter().next().unwrap_or((&zero, &zero));
      snapshot.spot_bid = price.clone();
    };

    snapshot.entry_percent = if snapshot.spot_ask != dec!(0) {
      ((snapshot.future_bid - snapshot.spot_ask) / snapshot.spot_ask) * dec!(100)
    } else {
      dec!(0)
    };

    snapshot.exit_percent = if snapshot.future_ask != dec!(0) {
      ((snapshot.spot_bid - snapshot.future_ask) / snapshot.future_ask) * dec!(100)
    } else {
      dec!(0)
    };

    let need_notification = snapshot.entry_percent > dec!(0) || snapshot.exit_percent > dec!(0);

    let symbol_cl = symbol.clone();

    if need_notification {
      let spot_exchange = arbitrage_cl.spot.exchange.clone();
      let future_exchange = arbitrage_cl.future.exchange.clone();
      let mut parsed_symbol = symbol_cl;
      if let Some((before, _)) = parsed_symbol.split_once(':') {
        parsed_symbol = before.to_string();
      }

      let notification = json!({
        "spot": spot_exchange,
        "future": future_exchange,
        "symbol": parsed_symbol,
        "entryPercent": snapshot.entry_percent,
        "exitPercent": snapshot.exit_percent
      });

      let json = serde_json::to_string(&notification).ok().unwrap();

      let _ = state.ws_tx.broadcast(json).await;
    }
  }

  Reenter {
    exchange: exchange.clone(),
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
    let future_symbol = format!("{}/{}:{}", item.future.base, item.future.quote, item.future.quote);

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
    ntex::time::sleep(Duration::from_millis(100)).await;

    tasks.push(spawn_live_calc(
      reenter.exchange,
      reenter.symbol,
      reenter.arbitrage.clone(),
      state.clone(),
    ));
  }
}
