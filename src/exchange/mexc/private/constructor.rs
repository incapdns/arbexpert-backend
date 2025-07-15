use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::atomic::AtomicBool};
use crate::exchange::mexc::MexcExchange;

impl MexcExchange {
  pub async fn with_credentials(api_key: String, secret: String, web_token: String) -> Self {
    let public = Self::new().await;

    Self {
      private: super::MexcExchangePrivate {
        subscribed_private_orders: false,
        inited_private_ws: Rc::new(AtomicBool::new(false)),
        api_key: Some(api_key.clone()),
        secret: Some(secret),
        web_token: Some(web_token),
        client: Some(super::Client {
          listen_key: None,
          private_ws: None,
          private_last_id: 0,
          private_pending: Rc::new(RefCell::new(HashMap::new())),
        }),
      },
      ..public
    }
  }
}