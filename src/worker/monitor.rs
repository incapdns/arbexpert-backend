use crate::Arbitrage;
use crate::exchange::binance::BinanceExchange;
use crate::worker::commands::StartMonitor;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use rust_decimal::dec;
use std::cell::UnsafeCell;
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
) -> Reenter {
  let order_book = exchange.watch_orderbook(symbol.clone()).await;

  let arbitrage_cl = arbitrage.clone();

  let snapshot = unsafe { &mut *arbitrage_cl.as_ref().snaphot.get() };

  if let Ok(ob) = order_book {
    let zero = dec!(0);
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

    snapshot.entry_percent = ((snapshot.future_bid - snapshot.spot_ask) / snapshot.spot_ask) * dec!(100);
    snapshot.exit_percent  = (snapshot.spot_bid - snapshot.future_ask) * dec!(100) / snapshot.future_ask;
  }

  Reenter {
    exchange: exchange.clone(),
    symbol,
    arbitrage: arbitrage_cl,
  }
}

pub async fn start_monitor(sm: StartMonitor, binance: Rc<BinanceExchange>) {
  let mut tasks = FuturesUnordered::new();

  for item in sm.items.iter() {
    let spot_symbol = item.spot.symbol.clone();
    let future_symbol = format!("{spot_symbol}:USDT");

    tasks.push(spawn_live_calc(
      binance.clone(),
      spot_symbol, //
      item.clone(),
    ));

    tasks.push(spawn_live_calc(
      binance.clone(),
      future_symbol,
      item.clone(),
    ));
  }

  while let Some(reenter) = tasks.next().await {
    tasks.push(spawn_live_calc(
      reenter.exchange,
      reenter.symbol,
      reenter.arbitrage.clone(),
    ));
  }
}
