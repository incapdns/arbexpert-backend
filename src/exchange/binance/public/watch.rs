use crate::base::exchange::assets::MarketType;
use crate::base::exchange::order::OrderBook;
use crate::exchange::binance::BinanceExchange;
use crate::exchange::binance::public::sub_client::BinanceSubClient;
use std::error::Error;
use std::rc::Rc;

impl BinanceExchange {
  pub async fn watch_orderbook(&mut self, symbol: &str) -> Result<Rc<OrderBook>, Box<dyn Error>> {
    let normalized = self.normalize_symbol(symbol);

    let sub_clients;
    let is_future = symbol.contains(':');
    let market = if is_future {
      sub_clients = &mut self.public.future_clients;
      MarketType::Future
    } else {
      sub_clients = &mut self.public.spot_clients;
      MarketType::Spot
    };

    let symbol = normalized.as_str();

    if let Some(client) = sub_clients
      .iter_mut()
      .find(|c| c.has_symbol(symbol)) 
    {
      return client.watch(symbol).await;
    }

    if let Some(client) = sub_clients
      .iter_mut()
      .find(|c| c.subscribed_count() < 5) 
    {
      return client.watch(symbol).await;
    }

    let mut new_sc = BinanceSubClient::new(self.utils.clone(), market);
    let book = new_sc.watch(symbol).await?;
    sub_clients.push(new_sc);
    Ok(book)
  }

  pub async fn unwatch_orderbook(&mut self, symbol: &str) -> Result<(), Box<dyn Error>> {
    let normalized = self.normalize_symbol(symbol);

    let is_future = symbol.contains(':');
    let sub_clients = if is_future {
      &mut self.public.future_clients
    } else {
      &mut self.public.spot_clients
    };

    let symbol = normalized.as_str();

    if let Some(pos) = sub_clients
      .iter()
      .position(|c| c.has_symbol(symbol)) 
    {
      let client = &mut sub_clients[pos];
      client.unwatch(symbol).await?;
      if client.subscribed_count() == 0 {
        sub_clients.remove(pos);
      }
    }

    Ok(())
  }
}
