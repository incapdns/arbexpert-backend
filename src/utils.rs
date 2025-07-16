use futures::join;

use crate::{base::exchange::Exchange, exchange::binance::BinanceExchange};

pub async fn setup_exchanges() -> Vec<Box<dyn Exchange>> {
  let (a,) = join!(BinanceExchange::new(),);

  vec![Box::new(a)]
}