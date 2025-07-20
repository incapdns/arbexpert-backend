use std::collections::BTreeMap;

use futures::join;

use crate::{
  base::exchange::Exchange,
  exchange::{binance::BinanceExchange, gate::GateExchange},
};

pub async fn setup_exchanges() -> Vec<Box<dyn Exchange>> {
  let (a, b) = join!(BinanceExchange::new(), GateExchange::new());

  vec![Box::new(a), Box::new(b)]
}

pub fn get_price<'a, K, V>(side: &'a BTreeMap<K, V>, default: &'a K) -> &'a K {
  side
    .iter()
    .next()
    .map(|(p, _)| p.clone())
    .unwrap_or(default)
}