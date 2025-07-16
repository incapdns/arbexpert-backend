use crate::{base::exchange::assets::Assets, exchange::binance::public::sub_client::BinanceSubClient};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

pub mod constructor;
pub mod sub_client;
pub mod watch;

pub struct BinanceExchangePublic {
  pub spot_clients: RefCell<Vec<Rc<BinanceSubClient>>>,
  pub future_clients: RefCell<Vec<Rc<BinanceSubClient>>>,
  pub assets: Option<Assets>,
  pub pairs: Rc<RefCell<HashMap<String, String>>>,
  pub time_offset_ms: i64,
}
