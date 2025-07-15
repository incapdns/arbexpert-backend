use crate::{base::exchange::OrderBook, exchange::mexc::MexcExchange};
use async_channel::{Receiver, Sender};
use rust_decimal::Decimal;
use send_wrapper::SendWrapper;
use serde::{Deserialize, Serialize};
use std::{
  collections::{HashMap, HashSet}, sync::{atomic::AtomicU32, Arc, Mutex}
};

type OneshotOrderbookSender = futures::channel::oneshot::Sender<OrderBook>;
type Symbol = String;
type WorkerId = u32;

pub struct GlobalState {
  pub last_id: AtomicU32,
  pub symbol_map: Mutex<HashMap<Symbol, WorkerId>>,
  pub worker_channels: Mutex<HashMap<WorkerId, Sender<Request>>>,
  pub next_worker: AtomicU32,
  pub exchanges: Vec<SendWrapper<MexcExchange>>
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Credentials {
  api_key: String,
  secret: String,
  web_token: String
}

#[derive(Serialize, Deserialize, Debug)]
pub struct StartArbitrage {
  symbol: String,
  quantity: Decimal,
  max_per_order: Decimal,
  entry_percent: Decimal,
  exit_percent: Decimal,
  index: i32,
  is_loop: bool,
  credentials: Credentials
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Request {
  Start(StartArbitrage)
}

pub async fn worker_loop(
  worker_id: WorkerId, 
  rx: Receiver<Request>,
  global_state: Arc<GlobalState>
) {
  let mut exchange = MexcExchange::new().await;

  while let Ok(req) = rx.recv().await {
    match req {
      Request::Start(command) => {
        
      }
    }
  }
}