use futures::join;

use crate::{base::exchange::Exchange, exchange::{binance::BinanceExchange, gate::GateExchange}};

pub async fn setup_exchanges() -> Vec<Box<dyn Exchange>> {
  let (a, b) = join!(
    BinanceExchange::new(),
    GateExchange::new()
  );

  vec![
    Box::new(a),
    Box::new(b)
  ]
}