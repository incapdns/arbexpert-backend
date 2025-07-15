use crate::{exchange::mexc::MexcExchange, worker::commands::Request};
use async_channel::Sender;
use send_wrapper::SendWrapper;
use std::{
  collections::{HashMap}, sync::{atomic::AtomicU32, Mutex}
};

pub type Symbol = String;
pub type WorkerId = u32;

pub struct GlobalState {
  pub last_id: AtomicU32,
  pub symbol_map: Mutex<HashMap<Symbol, WorkerId>>,
  pub worker_channels: Mutex<HashMap<WorkerId, Sender<Request>>>,
  pub next_worker: AtomicU32,
  pub exchanges: Vec<SendWrapper<MexcExchange>>
}
