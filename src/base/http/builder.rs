use crate::{base::http::generic::{DynamicIterator, HttpBody, HttpClient}, from_headers};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RequestError {
  #[error("Missing uri and method params")]
  MissingParams,
  #[error("Invalid response")]
  InvalidResponse,
}

pub struct HttpRequestBuilder<Client>
where
  Client: HttpClient,
{
  client: Client,
  method: Option<String>,
  url: Option<String>,
  headers: HashMap<String, String>,
  body: Option<Box<dyn HttpBody>>,
}

impl<Client> HttpRequestBuilder<Client>
where
  Client: HttpClient,
{
  pub fn new(client: Client, method: impl AsRef<str>, url: impl AsRef<str>) -> Self {
    Self {
      client,
      method: Some(method.as_ref().into()),
      url: Some(url.as_ref().into()),
      headers: HashMap::new(),
      body: None,
    }
  }

  pub fn set_header<K, V>(mut self, key: K, value: V) -> Self
  where
    K: AsRef<str>,
    V: AsRef<str>,
  {
    self
      .headers
      .insert(key.as_ref().to_string(), value.as_ref().to_string());
    self
  }

  pub fn set_body(mut self, body: impl HttpBody) -> Self {
    self.body = Some(Box::new(body));
    self
  }

  pub async fn do_request<'a>(self) -> Result<Client::Response, Box<dyn std::error::Error>>
  {
    let method = self.method.clone().ok_or(RequestError::MissingParams);
    let uri = self.url.clone().ok_or(RequestError::MissingParams);
    let client = self.client;

    client
      .request(method.unwrap(), uri.unwrap(), from_headers!(self.headers), self.body)
      .await
  }
}
