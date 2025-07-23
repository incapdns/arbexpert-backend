use crate::base::exchange::assets::MarketType;
use crate::base::exchange::sub_client::SharedBook;
use crate::exchange::binance::BinanceExchange;
use crate::exchange::binance::public::sub_client::BinanceSubClient;
use std::error::Error;
use std::rc::Rc;

impl BinanceExchange {
  pub async fn watch_orderbook(&self, symbol: String) -> Result<SharedBook, Box<dyn Error>> {
    let normalized = self.normalize_symbol(&symbol);
    let is_future = symbol.contains(':');

    let sub_clients = if is_future {
      &self.public.future_clients
    } else {
      &self.public.spot_clients
    };
    let market = if is_future {
      MarketType::Future
    } else {
      MarketType::Spot
    };
    let formatted = normalized.as_str();

    // 1) Se já houver client com esse symbol, usa ele
    if let Some(client) = {
      let guard = sub_clients.borrow();
      guard.iter().find(|c| c.has_symbol(formatted)).cloned()
    } {
      return client.watch(formatted).await;
    }

    // 2) Se houver client com < 7 subscriptions, usa ele
    if let Some(client) = {
      let guard = sub_clients.borrow();
      guard.iter().find(|c| c.subscribed_count() < 35).cloned()
    } {
      return client.watch(formatted).await;
    }

    // Empurra o novo client para o vetor
    let rc_new_sc = {
      let mut guard = sub_clients.borrow_mut();
      let new_sc = BinanceSubClient::new(self.utils.clone(),  self.public.time_offset_ms, market);
      let rc_new_sc = Rc::new(new_sc);
      guard.push(rc_new_sc.clone());
      rc_new_sc
    };

    let book = rc_new_sc.watch(formatted).await?;

    Ok(book)
  }

  pub async fn unwatch_orderbook(&self, symbol: &str) -> Result<(), Box<dyn Error>> {
    let normalized = self.normalize_symbol(symbol);

    let is_future = symbol.contains(':');
    let sub_clients = if is_future {
      &self.public.future_clients
    } else {
      &self.public.spot_clients
    };

    let formatted = normalized.as_str();

    if let Some(pos) = {
      sub_clients
        .borrow()
        .iter()
        .position(|c| c.has_symbol(formatted))
    } {
      let client = {
        let guard = sub_clients.borrow();
        guard[pos].clone()
      };

      client.unwatch(formatted).await?;
      if client.subscribed_count() == 0 {
        let mut guard = sub_clients.borrow_mut();
        guard.remove(pos);
      }
    }

    Ok(())
  }
}
