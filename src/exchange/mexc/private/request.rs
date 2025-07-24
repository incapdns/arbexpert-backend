use crate::base::exchange::error::ExchangeError;
use crate::base::http::generic::DynamicIterator;
use crate::exchange::mexc::{HmacSha256, MexcExchange};
use crate::from_headers;
use hmac::Mac;
use ntex::http::Method;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

impl MexcExchange {
  pub fn sign_request(&self, sorted: &BTreeMap<String, String>) -> Result<String, ExchangeError> {
    let secret = self
      .private
      .secret
      .as_ref()
      .ok_or(ExchangeError::MissingCredentials)?;

    let prehash = sorted
      .iter()
      .map(|(k, v)| format!("{k}={v}"))
      .collect::<Vec<_>>()
      .join("&");

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
      .map_err(|e| ExchangeError::ApiError(e.to_string()))?;
    mac.update(prehash.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
  }

  pub async fn request(
    &self,
    url: String,
    method: Method,
    mut params: HashMap<String, String>,
    body: Option<Value>,
  ) -> Result<Value, ExchangeError> {
    params.insert("recvWindow".into(), "10000".into());

    let local_time = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap()
      .as_millis() as i64
      + self.public.time_offset_ms;

    params.insert("timestamp".into(), local_time.to_string());

    // 2) Ordenar todos os params em BTreeMap para ASCII order
    let mut sorted = BTreeMap::new();
    for (k, v) in params.drain() {
      sorted.insert(k, v);
    }

    let signature = self.sign_request(&sorted)?;
    sorted.insert("signature".into(), signature);

    let final_query = sorted
      .iter()
      .map(|(k, v)| format!("{k}={v}"))
      .collect::<Vec<_>>()
      .join("&");
    let final_url = format!("{url}?{final_query}");

    let body_str = if let Some(b) = &body {
      serde_json::to_string(b).map_err(ExchangeError::JsonError)?
    } else {
      String::new()
    };

    let api_key = self
      .private
      .api_key
      .as_ref()
      .ok_or(ExchangeError::MissingCredentials)?;
    let mut headers = HashMap::new();
    headers.insert("X-MEXC-APIKEY".to_string(), api_key.clone());
    let client = &self.utils.http_client;

    let ref_headers = from_headers!(headers);

    let req = client.request(
      method.to_string(),
      final_url,
      ref_headers,
      if body_str.is_empty() {
        None
      } else {
        Some(Box::new(body_str))
      },
    );

    let mut resp = req
      .await
      .map_err(|e| ExchangeError::ApiError(e.to_string()))?;
    let status = resp.status();
    let bytes = resp
      .body()
      .await
      .map_err(|e| ExchangeError::ApiError(e.to_string()))?;
    let text =
      String::from_utf8(bytes.to_vec()).map_err(|e| ExchangeError::ApiError(e.to_string()))?;

    if status.is_success() {
      serde_json::from_str(&text).map_err(ExchangeError::JsonError)
    } else {
      let code = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("code").and_then(|c| c.as_i64()));
      let err = format!("API Error {status}: {text}");
      match code {
        Some(10000..=19999) => Err(ExchangeError::BadRequest(err)),
        Some(20000..=29999) => Err(ExchangeError::AuthenticationError(err)),
        Some(30000..=39999) => Err(ExchangeError::InvalidOrder(err)),
        _ => Err(ExchangeError::ApiError(err)),
      }
    }
  }
}