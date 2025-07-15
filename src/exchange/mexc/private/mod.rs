use crate::exchange::mexc::private::client::Client;
use std::{rc::Rc, sync::atomic::AtomicBool};

pub(super) struct MexcExchangePrivate {
  pub(super) subscribed_private_orders: bool,
  pub(super) inited_private_ws: Rc<AtomicBool>,
  pub(super) client: Option<Client>,
  pub(super) api_key: Option<String>,
  pub(super) secret: Option<String>,
  pub(super) web_token: Option<String>,
}

pub mod client;
pub mod constructor;
pub mod fetch;
pub mod request;
pub mod watch;

pub(super) fn empty_private() -> MexcExchangePrivate {
  MexcExchangePrivate {
    subscribed_private_orders: false,
    inited_private_ws: Rc::new(AtomicBool::new(false)),
    client: None,
    api_key: None,
    secret: None,
    web_token: None,
  }
}