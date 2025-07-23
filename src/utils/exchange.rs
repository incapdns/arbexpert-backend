use std::collections::BTreeMap;

use futures::join;

use crate::{
  base::exchange::Exchange,
  exchange::{binance::BinanceExchange, gate::GateExchange},
};

pub async fn setup_exchanges() -> Vec<Box<dyn Exchange>> {
  let (gate, binance) = join!(GateExchange::new(), BinanceExchange::new());

  vec![Box::new(gate)]
}

pub fn get_price<'a, K, V>(side: &'a BTreeMap<K, V>, default: &'a K) -> &'a K {
  side
    .iter()
    .next()
    .map(|(p, _)| p)
    .unwrap_or(default)
}