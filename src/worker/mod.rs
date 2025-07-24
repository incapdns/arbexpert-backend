use std::error::Error;
use std::sync::Arc;
use crate::base::exchange::Exchange;
use crate::utils::exchange::setup_exchanges;
use crate::worker::state::WorkerId;
use crate::{
  worker::{commands::Request, state::GlobalState},
};
use async_channel::Receiver;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use monitor::start_monitor;

pub mod commands;
pub mod monitor;
pub mod state;

async fn process_request(request: Request, exchanges: &Vec<Box<dyn Exchange>>, state: Arc<GlobalState>) {
  match request {
    Request::StartArbitrage(_) => {}
    Request::StartMonitor(command) => 
      start_monitor(command, exchanges, state).await,
  }
}

pub async fn worker_loop(
  worker_id: WorkerId,
  rx: Receiver<Request>,
  state: Arc<GlobalState>,
) -> Result<(), Box<dyn Error>> {
  println!("{worker_id} started !");

  let exchanges = setup_exchanges().await;

  let mut tasks = FuturesUnordered::new();

  loop {
    macros::select! {
      req = rx.recv() => {
        let req = req.map_err(Box::new);
        tasks.push(
          process_request(
            req.unwrap(),
            &exchanges,
            state.clone()
          )
        );
      },
      _ = tasks.next(), if !tasks.is_empty() => {},
    }
  }
}