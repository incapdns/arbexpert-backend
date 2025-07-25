use std::collections::BTreeMap;

use futures::join;

use crate::{
  base::exchange::Exchange,
  exchange::{binance::BinanceExchange, gate::GateExchange},
};

pub async fn setup_exchanges() -> Vec<Box<dyn Exchange>> {
  let (gate, binance) = join!(GateExchange::new(), BinanceExchange::new());

  vec![Box::new(gate),]
}

pub fn get_price<K: Clone, V: Clone>(side: &BTreeMap<K, V>) -> (K, V) {
  let (k, v) = side
    .iter()
    .next()
    .expect("no price");

  (k.clone(), v.clone())
}