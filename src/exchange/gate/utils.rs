use crate::{base::http::generic::HttpClient, exchange::gate::GateExchange};
use ntex::http::client::ClientResponse;

pub fn before(s: &str, character: char) -> &str {
  match s.find(character) {
    Some(pos) => &s[..pos],
    None => s,
  }
}

impl GateExchange {
  pub(super) fn normalize_symbol(&self, symbol: &str) -> String {
    let market_type = if symbol.contains(':') {
      "future"
    } else {
      "spot"
    };
    let formatted = before(symbol, ':').replace('/', "_");
    format!("{}@{}", formatted, market_type)
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
