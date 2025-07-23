use std::{cmp::Reverse, collections::BTreeMap};

use rust_decimal::{dec, Decimal};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
  pub id: String,
  pub symbol: String,
  pub side: String,
  pub amount: Decimal,
  pub price: Decimal,
  pub status: String,
  pub timestamp: u64,
  pub filled: Decimal,
  pub remaning: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderRequest {
  pub symbol: String,
  pub side: String,
  pub amount: Decimal,
  pub price: Decimal,
}

#[derive(Clone, Debug)]
pub struct OrderBook {
  pub bids: BTreeMap<Reverse<Decimal>, Decimal>, // preço -> quantidade
  pub asks: BTreeMap<Decimal, Decimal>, // preço -> quantidade
  pub update_id: u64,
}

impl Default for OrderBook {
  fn default() -> Self {
    Self {
      bids: BTreeMap::new(),
      asks: BTreeMap::new(),
      update_id: 0,
    }
  }
}

#[derive(Clone, Debug)]
pub struct OrderBookUpdate {
  pub bids: Vec<(Decimal, Decimal)>,
  pub asks: Vec<(Decimal, Decimal)>,
  pub first_update_id: u64,
  pub last_update_id: u64,
  pub full: bool
}

impl OrderBook {
  pub fn apply_update(&mut self, update: &OrderBookUpdate) {
    if update.full {
      self.asks.clear();
      self.bids.clear();
    }

    let bids_len = update.bids.len();
    let asks_len = update.asks.len();

    let zero = Decimal::ZERO;

    for (price, qty) in update.bids.iter() {
      let key = Reverse(price.clone());
      if qty.eq(&zero) {
        self.bids.remove(&key);
      } else {
        self.bids.insert(key, qty.clone());
      }
    }

    for (price, qty) in update.asks.iter() {
      if qty.eq(&zero) {
        self.asks.remove(price);
      } else {
        self.asks.insert(price.clone(), qty.clone());
      }
    }

    if bids_len > 0 || asks_len > 0 {
      self.update_id += 1;
    }

    if update.last_update_id > 0 {
      self.update_id = update.last_update_id;
    }
  }

  // Opcional: retorna os bids/asks como vetores ordenados
  pub fn get_bids(&self) -> Vec<(Reverse<Decimal>, Decimal)> {
    self.bids.iter().map(|(&p, &q)| (p, q)).collect()
  }

  pub fn get_asks(&self) -> Vec<(Decimal, Decimal)> {
    self.asks.iter().map(|(&p, &q)| (p, q)).collect()
  }
}