use std::error::Error;
use std::rc::Rc;
use std::sync::Arc;

use monitor::start_monitor;
use crate::worker::state::WorkerId;
use crate::{
  exchange::binance::BinanceExchange,
  worker::{commands::Request, state::GlobalState},
};
use async_channel::Receiver;
use futures::future::pending;
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt, select};

pub mod commands;
pub mod state;
pub mod monitor;

async fn process_request(request: Request, binance: Rc<BinanceExchange>) {
  match request {
    Request::StartArbitrage(_) => {}
    Request::StartMonitor(command) =>
      start_monitor(command, binance).await,
  }
}

pub async fn worker_loop(
  worker_id: WorkerId,
  rx: Receiver<Request>,
  _: Arc<GlobalState>,
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
        tasks.push(process_request(req, binance.clone()));
      },
      _ = next_task(&mut tasks).fuse() => {},
    };
  }
}
