use std::{
  collections::{HashMap, VecDeque},
  sync::{Arc, Mutex},
  time::{Duration, Instant},
};

use rust_decimal::Decimal;

use crate::base::exchange::order::{Order, OrderRequest};

#[derive(Clone, Debug, Default)]
pub struct Balance {
  pub spot: HashMap<String, Decimal>,
  pub future: HashMap<String, Decimal>,
}

#[derive(Debug)]
pub struct TimedOrder {
  order: Order,
  inserted_at: Instant,
}

#[derive(Debug, Default)]
pub struct SharedState {
  pub actual_balance: Balance,
  pub temporary_balance: Balance,
  pub orders: VecDeque<TimedOrder>,
}

#[derive(Clone)]
pub struct Account {
  inner: Arc<Mutex<SharedState>>,
  credentials: serde_json::Value
}

impl Account {
  pub fn new(credentials: serde_json::Value) -> Self {
    Self {
      inner: Arc::new(Mutex::new(SharedState::default())),
      credentials
    }
  }

  pub fn get_credentials(&self) -> &serde_json::Value {
    &self.credentials
  }

  fn order_exists(orders: &VecDeque<TimedOrder>, id: &str) -> bool {
    orders.iter().any(|o| o.order.id == id)
  }

  pub fn insert_order(&self, order: Order) {
    let mut state = self.inner.lock().unwrap();

    // Clean up old orders
    let now = Instant::now();
    while let Some(front) = state.orders.front() {
      if now.duration_since(front.inserted_at) > Duration::from_secs(30) {
        state.orders.pop_front();
      } else {
        break;
      }
    }

    if Self::order_exists(&state.orders, &order.id) {
      return;
    }

    state.orders.push_back(TimedOrder {
      order: order.clone(),
      inserted_at: now,
    });

    Self::apply_order(
      &mut state.actual_balance,
      &OrderRequest {
        amount: order.amount,
        price: order.price,
        side: order.side,
        symbol: order.symbol,
      },
    );
  }

  pub fn simulate_order(&self, order: &OrderRequest) {
    let mut state = self.inner.lock().unwrap();
    Self::apply_order(&mut state.temporary_balance, order);
  }

  fn apply_order(balance: &mut Balance, order: &OrderRequest) {
    let is_future = order.symbol.contains(':');
    let balance_map = if is_future {
      &mut balance.future
    } else {
      &mut balance.spot
    };

    let entry = balance_map.entry("USDT".to_string()).or_default();
    let value = order.amount * order.price;

    if is_future {
      match order.side.to_lowercase().as_str() {
        "sell" => *entry -= value, // open short
        "buy" => *entry += value,  // close short
        _ => eprintln!("Unknown future side: {}", order.side),
      }
    } else {
      match order.side.to_lowercase().as_str() {
        "buy" => *entry -= value,
        "sell" => *entry += value,
        _ => eprintln!("Unknown spot side: {}", order.side),
      }
    }
  }

  pub fn get_actual_balance(&self) -> Balance {
    let state = self.inner.lock().unwrap();
    state.actual_balance.clone()
  }

  pub fn get_temporary_balance(&self) -> Balance {
    let state = self.inner.lock().unwrap();
    state.temporary_balance.clone()
  }

  pub fn get_orders(&self) -> Vec<Order> {
    let state = self.inner.lock().unwrap();
    state.orders.iter().map(|t| t.order.clone()).collect()
  }

  pub fn reset_temporary_balance(&self) {
    let mut state = self.inner.lock().unwrap();
    state.temporary_balance = state.actual_balance.clone();
  }
}