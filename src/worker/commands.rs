use serde::{Deserializer, ser::SerializeSeq};
use std::{cell::UnsafeCell, sync::Arc};

use rust_decimal::{Decimal, dec};
use serde::{Deserialize, Serialize};

use crate::{Arbitrage, ArbitrageResult};

#[derive(Serialize, Deserialize, Debug)]
pub struct Credentials {
  api_key: String,
  secret: String,
  web_token: String,
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
  credentials: Credentials,
}

#[derive(Debug)]
pub struct StartMonitor {
  pub items: Arc<Vec<Arc<Arbitrage>>>,
}

impl Serialize for StartMonitor {
  fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
  where
    S: serde::Serializer,
  {
    let mut seq = serializer.serialize_seq(Some(self.items.len()))?;

    for item in self.items.iter() {
      let exportable = item.as_ref();
      seq.serialize_element(exportable)?;
    }

    seq.end()
  }
}

impl<'de> Deserialize<'de> for StartMonitor {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    let inputs: Vec<Arbitrage> = Deserialize::deserialize(deserializer)?;

    let arbitrages: Vec<Arc<Arbitrage>> = inputs
      .into_iter()
      .map(|input| {
        Arc::new(Arbitrage {
          spot: input.spot,
          future: input.future,
          result: UnsafeCell::new(ArbitrageResult {
            entry_percent: dec!(0),
            exit_percent: dec!(0),
          }),
        })
      })
      .collect();

    Ok(StartMonitor {
      items: Arc::new(arbitrages),
    })
  }
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Request {
  StartArbitrage(StartArbitrage),
  StartMonitor(StartMonitor),
}
