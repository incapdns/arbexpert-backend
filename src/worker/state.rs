use crate::{worker::commands::Request, Arbitrage};
use async_channel::Sender;
use std::{
  collections::HashMap, sync::{atomic::{AtomicBool, AtomicU32}, Arc, Mutex, RwLock}
};

pub type Symbol = String;
pub type WorkerId = u32;

pub struct GlobalState {
  pub last_id: AtomicU32,
  pub symbol_map: Mutex<HashMap<Symbol, WorkerId>>,
  pub worker_channels: Mutex<HashMap<WorkerId, Sender<Request>>>,
  pub next_worker: AtomicU32,
  pub started: AtomicBool,
  pub arbitrages: Arc<RwLock<Vec<Arc<Arbitrage>>>>,
  pub ws_tx: async_broadcast::Sender<String>,
}