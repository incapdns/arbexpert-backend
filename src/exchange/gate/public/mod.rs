use crate::{base::exchange::assets::Assets, exchange::gate::public::sub_client::GateSubClient};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

pub mod constructor;
pub mod sub_client;
pub mod watch;

pub struct GateExchangePublic {
  pub spot_clients: RefCell<Vec<Rc<GateSubClient>>>,
  pub future_clients: RefCell<Vec<Rc<GateSubClient>>>,
  pub assets: Option<Assets>,
  pub pairs: Rc<RefCell<HashMap<String, String>>>,
  pub time_offset_ms: i64,
}
