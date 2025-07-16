use crate::base::exchange::order::OrderBook;
use crate::exchange::mexc::MexcExchange;
use crate::exchange::mexc::public::sub_client::SubClient;
use std::error::Error;
use std::rc::Rc;

impl MexcExchange {
  pub async fn watch_orderbook(&mut self, symbol: &str) -> Result<Rc<OrderBook>, Box<dyn Error>> {
    let normalized = self.normalize_symbol(symbol);
    let symbol = normalized.as_str();

    if let Some(idx) = self
      .public
      .sub_clients
      .iter()
      .position(|c| c.has_symbol(symbol))
    {
      return self.public.sub_clients[idx].watch(symbol).await;
    }

    if let Some(client) = self
      .public
      .sub_clients
      .iter_mut()
      .find(|c| c.subscribed_count() < 7)
    {
      return client.watch(symbol).await;
    }

    let mut new_sc = SubClient::new("wss://wbs-api.mexc.com/ws");
    new_sc.connect().await?;
    let book = new_sc.watch(symbol).await?;
    self.public.sub_clients.push(new_sc);
    Ok(book)
  }

  pub async fn unwatch_orderbook(&mut self, symbol: &str) -> Result<(), Box<dyn Error>> {
    let normalized = self.normalize_symbol(symbol);
    let symbol = normalized.as_str();
    if let Some(pos) = self
      .public
      .sub_clients
      .iter()
      .position(|c| c.has_symbol(symbol))
    {
      let client = &mut self.public.sub_clients[pos];
      client.unwatch(symbol).await?;
      if client.subscribed_count() == 0 {
        self.public.sub_clients.remove(pos);
      }
    }
    Ok(())
  }
}
