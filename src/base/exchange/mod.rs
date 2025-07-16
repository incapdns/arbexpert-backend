use std::error::Error;
use crate::base::exchange::{assets::Assets, error::ExchangeError, sub_client::SharedBook};
use async_trait::async_trait;

pub mod order;
pub mod error;
pub mod account;
pub mod sub_client;
pub mod assets;


#[async_trait(?Send)]
pub trait Exchange {
  async fn watch_orderbook(
    &self,
    symbol: String,
  ) -> Result<SharedBook, Box<dyn Error>>;

  async fn fetch_assets(&mut self) -> Result<Assets, ExchangeError>;

  async fn load_assets(&mut self) -> Result<Assets, ExchangeError>;

  async fn sync_time(&mut self) -> Result<(), ExchangeError>;

  fn name(&self) -> String;

  fn assets(&self) -> Option<&Assets>;
}