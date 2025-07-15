use crate::base::exchange::order::Order;
use ntex::channel::mpsc::Sender;
use ntex::channel::{Canceled, oneshot};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Debug)]
pub struct CatchReturn {
  pub orders: Vec<Rc<Order>>,
  pub nonce: usize,
}

pub struct OrderCatchState {
  pub orders: Vec<(usize, Rc<Order>)>,
  pub nonce: usize,
  pub wakers: Vec<oneshot::Sender<()>>,
}

pub struct OrderCatch {
  pub state: Rc<RefCell<OrderCatchState>>,
  pub symbol: String,
  pub id: u32,
  pub pending_map: Rc<RefCell<HashMap<String, Vec<(u32, Sender<Rc<Order>>)>>>>,
}

impl OrderCatch {
  pub async fn next(&self, last_nonce: usize) -> Result<CatchReturn, Canceled> {
    {
      let state = self.state.borrow();
      if last_nonce < state.nonce {
        let orders = state
          .orders
          .iter()
          .filter(|(n, _)| *n > last_nonce)
          .map(|(_, o)| o.clone())
          .collect::<Vec<_>>();

        return Ok(CatchReturn {
          nonce: state.nonce,
          orders,
        });
      }
    }

    // Aguarda nova ordem
    let (tx, rx) = oneshot::channel();
    {
      let mut state = self.state.borrow_mut();
      state.wakers.push(tx);
    }

    rx.recv().await?;

    let state = self.state.borrow();
    let orders = state
      .orders
      .iter()
      .filter(|(n, _)| *n > last_nonce)
      .map(|(_, o)| o.clone())
      .collect::<Vec<_>>();

    Ok(CatchReturn {
      nonce: state.nonce,
      orders,
    })
  }
}

impl Drop for OrderCatch {
  fn drop(&mut self) {
    if Rc::strong_count(&self.state) <= 2 {
      let mut borrow = self.pending_map.borrow_mut();

      let symbol = self.symbol.replace('/', "").to_uppercase();

      let subscriptions = borrow.get_mut(&symbol).unwrap();

      if let Some(index) = subscriptions.iter().position(|(id, _)| *id == self.id) {
        subscriptions.remove(index);
      }
    }
  }
}
