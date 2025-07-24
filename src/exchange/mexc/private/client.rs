use crate::{
  base::{exchange::order::Order, ws::client::WsClient},
};
use ntex::channel::mpsc::Sender;
use std::{
  cell::RefCell,
  collections::HashMap,
  rc::Rc,
};

pub(super) struct Client {
  pub(super) listen_key: Option<String>,
  pub(super) private_ws: Option<WsClient>,
  pub(super) private_last_id: u32,
  pub(super) private_pending: Rc<RefCell<HashMap<String, Vec<(u32, Sender<Rc<Order>>)>>>>,
}