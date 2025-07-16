use ntex::http::client::ClientResponse;
use crate::{base::http::generic::HttpClient, exchange::binance::BinanceExchange};

fn before(s: &str) -> &str {
  match s.find(':') {
    Some(pos) => &s[..pos],
    None => s,
  }
}

impl BinanceExchange {
  pub(super) fn normalize_symbol(&self, symbol: &str) -> String {
    let result = symbol.replace('/', "").to_uppercase();
    let result = before(&result);
    let mut borrow = self.public.pairs.borrow_mut();
    borrow.insert(result.to_string(), symbol.to_string());
    result.to_string()
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