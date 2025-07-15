use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

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