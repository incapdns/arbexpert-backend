use ntex::http::client::ClientResponse;
use crate::{base::http::generic::HttpClient, exchange::binance::BinanceExchange};

pub fn before(s: &str, character: char) -> &str {
  match s.find(character) {
    Some(pos) => &s[..pos],
    None => s,
  }
}

impl BinanceExchange {
  pub(super) fn normalize_symbol(&self, symbol: &str) -> String {
    let market_type = if symbol.contains(':') {
      "future"
    } else {
      "spot"
    };
    let formatted = before(symbol, ':').replace('/', "");
    format!("{}@{}", formatted, market_type)
  }
}

pub struct BinanceExchangeUtils {
  pub http_client: Box<dyn HttpClient<Response = ClientResponse>>,
}

impl BinanceExchangeUtils {
  pub fn new<C>(http_client: C) -> Self
  where
    C: HttpClient<Response = ClientResponse> + 'static,
  {
    Self {
      http_client: Box::new(http_client),
    }
  }
}