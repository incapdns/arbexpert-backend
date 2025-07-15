use std::{collections::HashMap, mem, time::{Duration, SystemTime, UNIX_EPOCH}};
use ntex::http::Method;
use serde_json::Value;
use crate::{base::exchange::error::ExchangeError, exchange::mexc::MexcExchange};

impl MexcExchange {
  pub async fn fetch_last_orders(
    &mut self,
    symbol: &str,
    last: Duration,
  ) -> Result<Vec<Value>, ExchangeError> {
    let url = "https://api.mexc.com/api/v3/allOrders".to_string();
    let mut params = HashMap::new();
    params.insert("symbol".to_string(), self.normalize_symbol(symbol));

    let now = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map_err(|e| ExchangeError::ApiError(e.to_string()))?
      .as_millis() as i64;

    let start_time = now + self.public.time_offset_ms;

    let start_time = start_time
      .checked_sub(last.as_millis() as i64)
      .ok_or_else(|| ExchangeError::ApiError("Duration is too long".to_string()))?
      .to_string();

    params.insert("startTime".into(), start_time);
    params.insert("limit".into(), 50.to_string());

    let mut result = self.request(url, Method::GET, params, None).await?;

    let array = result.as_array_mut().map(|arr| mem::take(arr));

    if let None = array {
      return Err(ExchangeError::ApiError(
        "Unexpected response format".to_string(),
      ));
    }

    Ok(array.unwrap())
  }
}
