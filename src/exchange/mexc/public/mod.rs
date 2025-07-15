use std::{cell::RefCell, collections::HashMap, rc::Rc};
use crate::{exchange::mexc::{public::{sub_client::SubClient, symbols::SymbolEntry}}};

pub(super) struct MexcExchangePublic {
  pub(super) sub_clients: Vec<SubClient>,
  pub(super) markets: Option<Rc<HashMap<String, String>>>,
  pub(super) pairs: Rc<RefCell<HashMap<String, String>>>,
  pub(super) symbols: HashMap<String, SymbolEntry>,
  pub(super) time_offset_ms: i64,
}

pub mod constructor;
pub mod sub_client;
pub mod symbols;
pub mod watch;
