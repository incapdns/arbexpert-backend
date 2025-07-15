use crate::{base::exchange::assets::Assets, exchange::binance::public::sub_client::BinanceSubClient};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

pub mod constructor;
pub mod sub_client;
pub mod watch;

pub(super) struct BinanceExchangePublic {
  pub(super) spot_clients: Vec<BinanceSubClient>,
  pub(super) future_clients: Vec<BinanceSubClient>,
  pub(super) assets: Option<Assets>,
  pub(super) pairs: Rc<RefCell<HashMap<String, String>>>,
  pub(super) time_offset_ms: i64,
}
