use std::rc::Rc;
use crate::exchange::binance::{public::BinanceExchangePublic, utils::BinanceExchangeUtils};

pub mod utils;
pub mod public;

pub struct BinanceExchange {
  //private: BinanceExchangePrivate,
  public: BinanceExchangePublic,
  utils: Rc<BinanceExchangeUtils>
}