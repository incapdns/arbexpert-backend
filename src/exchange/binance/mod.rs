use crate::{
  base::exchange::{assets::Assets, Exchange},
  exchange::binance::{public::BinanceExchangePublic, utils::BinanceExchangeUtils},
};
use async_trait::async_trait;
use std::rc::Rc;

pub mod public;
pub mod utils;

pub struct BinanceExchange {
  //private: BinanceExchangePrivate,
  pub public: BinanceExchangePublic,
  pub utils: Rc<BinanceExchangeUtils>,
}

#[async_trait(?Send)]
impl Exchange for BinanceExchange {
  async fn watch_orderbook(
    &self,
    symbol: String,
  ) -> Result<crate::base::exchange::sub_client::SharedBook, Box<dyn std::error::Error>> {
    BinanceExchange::watch_orderbook(self, symbol).await
  }

  async fn fetch_assets(
    &mut self,
  ) -> Result<crate::base::exchange::assets::Assets, crate::base::exchange::error::ExchangeError>
  {
    BinanceExchange::fetch_assets(self).await
  }

  async fn load_assets(
    &mut self,
  ) -> Result<crate::base::exchange::assets::Assets, crate::base::exchange::error::ExchangeError>
  {
    BinanceExchange::load_assets(self).await
  }

  async fn sync_time(&mut self) -> Result<(), crate::base::exchange::error::ExchangeError> {
    BinanceExchange::sync_time(self).await
  }

  fn name(&self) -> String {
    BinanceExchange::name(self)
  }

  fn assets(&self) -> Option<&Assets> {
    self.public.assets.as_ref()
  }
}