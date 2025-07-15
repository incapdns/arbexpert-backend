use std::sync::Arc;

use crate::{exchange::{binance::BinanceExchange, mexc::MexcExchange}, worker::{commands::Request, state::GlobalState}};
use async_channel::Receiver;
use crate::worker::state::WorkerId;

pub mod state;
pub mod commands;

pub async fn worker_loop(
  worker_id: WorkerId, 
  rx: Receiver<Request>,
  global_state: Arc<GlobalState>
) {
  let binance = BinanceExchange::new().await;

  while let Ok(req) = rx.recv().await {
    match req {
      Request::Start(command) => {
        
      }
    }
  }
}