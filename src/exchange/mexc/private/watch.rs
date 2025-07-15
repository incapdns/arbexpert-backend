use std::{cell::RefCell, collections::HashMap, error::Error, mem, rc::Rc, sync::atomic::Ordering};

use ntex::http::Method;
use prost::Message;
use rust_decimal::dec;
use serde_json::{Value, json};

use crate::{
  base::{
    exchange::{error::ExchangeError, order::Order},
    http::generic::DynamicIterator,
    ws::client::{WsClient, WsOptions},
  },
  exchange::{
    catch::{OrderCatch, OrderCatchState},
    mexc::{
      MexcExchange, MexcExchangeUtils,
      mexc_proto::{PushDataV3ApiWrapper, push_data_v3_api_wrapper::Body},
    },
  },
  from_headers,
};

impl MexcExchange {
  fn release_private_ws(api_key: String, listen_key: String, client: Rc<MexcExchangeUtils>) {
    let url = format!(
      "https://api.mexc.com/api/v3/userDataStream?listenKey={}",
      listen_key
    );

    ntex::rt::spawn(async move {
      let mut headers = HashMap::new();
      headers.insert("X-MEXC-APIKEY".to_string(), api_key);

      let _ = client
        .http_client
        .request("DELETE".to_string(), url, from_headers!(headers), None)
        .await;
    });
  }

  async fn init_private_ws(&mut self) -> Result<(), Box<dyn Error>> {
    let symbol_cl = self.public.pairs.clone();
    let listen_key = self.create_listen_key();
    let listen_key = listen_key.await?;

    let client = self.private.client.as_mut().unwrap();

    let (pending1, pending2, pending3) = (
      client.private_pending.clone(),
      client.private_pending.clone(),
      client.private_pending.clone(),
    );

    let (inited1, inited2) = (
      self.private.inited_private_ws.clone(),
      self.private.inited_private_ws.clone(),
    );

    let (listen_key1, listen_key2) = (listen_key.clone(), listen_key.clone());

    let (api_key1, api_key2) = (
      self.private.api_key.clone().unwrap(),
      self.private.api_key.clone().unwrap(),
    );

    let (utils1, utils2) = (self.utils.clone(), self.utils.clone());

    let mut ws = WsClient::new(
      format!("wss://wbs-api.mexc.com/ws?listenKey={}", listen_key),
      Box::new(|_: String| async {}),
      Box::new(move |_| {
        Self::release_private_ws(api_key1.clone(), listen_key1.clone(), utils1.clone());
        inited1.store(false, Ordering::Relaxed);
        pending1.borrow_mut().clear();
      }),
      Box::new(move || {
        Self::release_private_ws(api_key2, listen_key2, utils2);
        inited2.store(false, Ordering::Relaxed);
        pending2.borrow_mut().clear();
      }),
      Box::new(|| {}),
      // no ensure_private_ws, no on_binary:
      Box::new(move |bin: Vec<u8>| {
        let symbol_cl = symbol_cl.clone();
        let pending3 = pending3.clone();
        async move {
          if let Ok(wrapper) = PushDataV3ApiWrapper::decode(&*bin) {
            if wrapper.symbol.is_none() {
              return;
            }

            let normalized_symbol = wrapper.symbol.clone().unwrap();

            let symbol = symbol_cl.borrow().get(&normalized_symbol).cloned().unwrap();

            if wrapper.channel.starts_with("spot@private.orders.v3.api.pb") {
              let Some(Body::PrivateOrders(order)) = wrapper.body else {
                return;
              };

              let order = Rc::new(Order {
                id: order.id,
                amount: order.amount.parse().unwrap_or_else(|_| dec!(0)),
                price: order.price.parse().unwrap(),
                side: if order.trade_type == 1 {
                  "buy".to_string()
                } else {
                  "sell".to_string()
                },
                symbol: symbol.clone(),
                status: match order.status {
                  1 => "open".to_string(),
                  2 => "open".to_string(),
                  3 => "filled".to_string(),
                  4 => "canceled".to_string(),
                  _ => "unknown".to_string(),
                },
                timestamp: 0,
                filled: order.remain_quantity.parse().unwrap(),
                remaning: order.cumulative_quantity.parse().unwrap(),
              });

              let mut subscriptions = {
                let mut map = pending3.borrow_mut();
                let vec = map.get_mut(&normalized_symbol).unwrap();
                mem::take(vec)
              };

              for (_, tx) in &subscriptions {
                let _ = tx.send(order.clone());
              }

              let mut map = pending3.borrow_mut();
              let vec = map.get_mut(&normalized_symbol);
              let vec = vec.unwrap();

              vec.append(&mut subscriptions);
            }
          }
        }
      }),
      WsOptions::default(),
    );

    ws.connect().await?;

    client.private_ws = Some(ws);

    Ok(())
  }

  async fn ensure_private_ws(&mut self) -> Result<(), Box<dyn Error>> {
    if self.private.inited_private_ws.load(Ordering::Relaxed) {
      return Ok(());
    }

    self.init_private_ws().await?;

    self
      .private
      .inited_private_ws
      .store(true, Ordering::Relaxed);

    Ok(())
  }

  pub async fn catch_orders(&mut self, symbol: &str) -> Result<OrderCatch, Rc<Box<dyn Error>>> {
    self.ensure_private_ws().await?;

    let client = self.private.client.as_ref().unwrap();

    if !self.private.subscribed_private_orders {
      self.private.subscribed_private_orders = true;

      let msg = json!({
        "method": "SUBSCRIPTION",
        "params": ["spot@private.orders.v3.api.pb"]
      })
      .to_string();

      if let Some(ws) = &client.private_ws {
        ws.send(msg).await?;
      }
    }

    let state = Rc::new(RefCell::new(OrderCatchState {
      orders: Vec::new(),
      nonce: 0,
      wakers: Vec::new(),
    }));

    let normalized = self.normalize_symbol(symbol);
    let client = self.private.client.as_mut().unwrap();

    let id = client.private_last_id + 1;
    client.private_last_id = id;

    let (tx, rx) = ntex::channel::mpsc::channel();
    client
      .private_pending
      .borrow_mut()
      .entry(normalized.clone())
      .or_insert_with(Vec::new)
      .push((id, tx));

    let state_clone = state.clone();
    ntex::rt::spawn(async move {
      while let Some(order) = rx.recv().await {
        let mut state = state_clone.borrow_mut();
        let nonce = state.nonce + 1;
        state.nonce = nonce;
        state.orders.push((nonce, order));

        let wakers = mem::take(&mut state.wakers);
        drop(state);

        for waker in wakers {
          let _ = waker.send(());
        }
      }
    });

    Ok(OrderCatch {
      state,
      symbol: symbol.to_string(),
      id,
      pending_map: client.private_pending.clone(),
    })
  }

  pub async fn create_listen_key(&mut self) -> Result<String, ExchangeError> {
    let url = "https://api.mexc.com/api/v3/userDataStream".to_string();

    let api_key = self
      .private
      .api_key
      .as_ref()
      .ok_or(ExchangeError::MissingCredentials)?;

    let mut headers_map = HashMap::new();
    headers_map.insert("X-MEXC-APIKEY".to_string(), api_key.clone());

    let json = self
      .request(url, Method::POST, headers_map, None)
      .await
      .map_err(|e| ExchangeError::ApiError(format!("Listen key error: {}", e)))?;

    let client = self.private.client.as_mut().unwrap();

    if let Some(listen_key) = json.get("listenKey").and_then(Value::as_str) {
      client.listen_key = Some(listen_key.to_string());
      Ok(listen_key.to_string())
    } else {
      Err(ExchangeError::ApiError(format!(
        "Missing listenKey in response: {}",
        json
      )))
    }
  }
}
