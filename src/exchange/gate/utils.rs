use ntex::http::client::ClientResponse;
use crate::{base::http::generic::HttpClient, exchange::gate::GateExchange};

fn before(s: &str) -> &str {
  match s.find(':') {
    Some(pos) => &s[..pos],
    None => s,
  }
}

impl GateExchange {
  pub(super) fn normalize_symbol(&self, symbol: &str) -> String {
    let result = symbol.replace('/', "_").to_uppercase();
    let result = before(&result);
    let mut borrow = self.public.pairs.borrow_mut();
    borrow.insert(result.to_string(), symbol.to_string());
    result.to_string()
  }
}

pub struct GateExchangeUtils {
  pub http_client: Box<dyn HttpClient<Response = ClientResponse>>,
}

impl GateExchangeUtils {
  pub fn new<C>(http_client: C) -> Self
  where
    C: HttpClient<Response = ClientResponse> + 'static,
  {
    Self {
      http_client: Box::new(http_client),
    }
  }
}