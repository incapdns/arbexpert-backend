use std::error::Error;
use std::rc::Rc;
use std::sync::Arc;

use crate::worker::state::WorkerId;
use crate::{
  exchange::binance::BinanceExchange,
  worker::{commands::Request, state::GlobalState},
};
use async_channel::Receiver;
use futures::future::pending;
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt, select};
use monitor::start_monitor;

pub mod commands;
pub mod monitor;
pub mod state;

async fn process_request(request: Request, binance: Rc<BinanceExchange>, state: Arc<GlobalState>) {
  match request {
    Request::StartArbitrage(_) => {}
    Request::StartMonitor(command) => 
      start_monitor(command, binance, state).await,
  }
}

pub async fn worker_loop(
  worker_id: WorkerId,
  rx: Receiver<Request>,
  state: Arc<GlobalState>,
) -> Result<(), Box<dyn Error>> {
  println!("{} started !", worker_id);

  let binance = BinanceExchange::new().await;
  let binance = Rc::new(binance);

  let mut tasks = FuturesUnordered::new();

  async fn next_task<F: Future>(tasks: &mut FuturesUnordered<F>) -> Option<F::Output> {
    if tasks.len() > 0 {
      tasks.next().await
    } else {
      pending().await
    }
  }

  loop {
    select! {
      req = rx.recv().fuse() => {
        let req = req.map_err(|err| Box::new(err))?;
        tasks.push(
          process_request(
            req,
            binance.clone(),
            state.clone()
          )
        );
      },
      _ = next_task(&mut tasks).fuse() => {},
    };
  }
}
