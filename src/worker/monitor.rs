use crate::exchange::binance::BinanceExchange;
use crate::worker::commands::StartMonitor;
use futures::future::pending;
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt, select};
use std::rc::Rc;

pub struct Reenter {
  pub exchange: Rc<BinanceExchange>,
  pub symbol: String,
}

async fn spawn_live_calc(exchange: Rc<BinanceExchange>, symbol: String) -> Reenter {
  let _ = exchange.watch_orderbook(symbol.clone()).await;

  Reenter {
    exchange: exchange.clone(),
    symbol,
  }
}

pub async fn start_monitor(sm: StartMonitor, binance: Rc<BinanceExchange>) {
  let mut tasks = FuturesUnordered::new();
  
  for item in sm.items.iter() {
    let spot_symbol = item.spot.symbol.clone();
    let future_symbol = format!("{spot_symbol}:USDT");

    tasks.push(spawn_live_calc(binance.clone(), spot_symbol));
    tasks.push(spawn_live_calc(binance.clone(), future_symbol));
  }

  while let Some(reenter) = tasks.next().await {
    tasks.push(spawn_live_calc(reenter.exchange, reenter.symbol));
  }
}