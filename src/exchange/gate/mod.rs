use crate::{
  base::exchange::{assets::Assets, Exchange},
  exchange::gate::{public::GateExchangePublic, utils::GateExchangeUtils},
};
use async_trait::async_trait;
use std::rc::Rc;

pub mod public;
pub mod utils;

pub struct GateExchange {
  //private: GateExchangePrivate,
  pub public: GateExchangePublic,
  pub utils: Rc<GateExchangeUtils>,
}

#[async_trait(?Send)]
impl Exchange for GateExchange {
  async fn watch_orderbook(
    &self,
    symbol: String,
  ) -> Result<crate::base::exchange::sub_client::SharedBook, Box<dyn std::error::Error>> {
    GateExchange::watch_orderbook(self, symbol).await
  }

  async fn fetch_assets(
    &mut self,
  ) -> Result<crate::base::exchange::assets::Assets, crate::base::exchange::error::ExchangeError>
  {
    GateExchange::fetch_assets(self).await
  }

  async fn load_assets(
    &mut self,
  ) -> Result<crate::base::exchange::assets::Assets, crate::base::exchange::error::ExchangeError>
  {
    GateExchange::load_assets(self).await
  }

  async fn sync_time(&mut self) -> Result<(), crate::base::exchange::error::ExchangeError> {
    GateExchange::sync_time(self).await
  }

  fn name(&self) -> String {
    GateExchange::name(self)
  }

  fn assets(&self) -> Option<&Assets> {
    self.public.assets.as_ref()
  }
}