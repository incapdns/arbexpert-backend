use ntex::http::client::ClientResponse;

use crate::{base::http::generic::HttpClient, exchange::mexc::MexcExchange};

impl MexcExchange {
  pub(super) fn normalize_symbol(&mut self, symbol: &str) -> String {
    let result = symbol.replace('/', "").to_uppercase();
    let mut borrow = self.public.pairs.borrow_mut();
    borrow.insert(result.to_string(), symbol.to_string());
    result
  }
}

pub(super) struct MexcExchangeUtils {
  pub(super) http_client: Box<dyn HttpClient<Response = ClientResponse>>,
}

impl MexcExchangeUtils {
  pub(super) fn new<C>(http_client: C) -> Self
  where
    C: HttpClient<Response = ClientResponse> + 'static,
  {
    Self {
      http_client: Box::new(http_client),
    }
  }
}
