use crate::base::http::generic::{DynamicIterator, HttpBody, HttpClient};
use ntex::http::client::Connector;
use ntex::http::{Client, Method, Version, client::ClientResponse};
use std::{error::Error, pin::Pin, time::Duration};
use thiserror::Error;

#[derive(Debug, Error)]
#[error("Invalid body")]
pub struct BodyError();

pub struct NtexHttpClient {
  client: Client,
}

impl Default for NtexHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl NtexHttpClient {
  pub fn new() -> Self {
    Self {
      client: Client::build()
        .connector(
          Connector::default()
            .timeout(Duration::from_secs(15))
            .finish(),
        )
        .finish(),
    }
  }
}

macro_rules! process_downgrade {
  (init, $type:ty, $http_body:ident, $backup:ident) => {{
    let result = $http_body.downcast::<$type>();
    if let Ok(result) = result {
      Some(result)
    } else {
      $backup = Some(result.err().unwrap());
      None
    }
  }};

  (resume, $type:ty, $backup:ident) => {{
    let result = $backup.take().unwrap().downcast::<$type>();
    if let Ok(result) = result {
      Some(result)
    } else {
      $backup = Some(result.err().unwrap());
      None
    }
  }};
}

impl HttpClient for NtexHttpClient {
  type Response = ClientResponse;

  fn request<'t, 'b>(
    &'t self,
    method: String,
    uri: String,
    mut headers: DynamicIterator<'t, (&'t dyn AsRef<str>, &'t dyn AsRef<str>)>,
    mut body: Option<Box<dyn HttpBody + 'b>>,
  ) -> Pin<Box<dyn Future<Output = Result<Self::Response, Box<dyn std::error::Error>>> + 't>> {
    let client = self.client.clone();

    Box::pin(async move {
      let method = method.parse::<Method>()?;

      let mut req = client
        .request(method, uri)
        .version(Version::HTTP_2)
        .timeout(Duration::from_secs(10));

      for (k, v) in &mut headers {
        req = req.header(k.as_ref(), v.as_ref());
      }

      if let Some(http_body) = body.take() {
        let mut backup: Option<Box<dyn HttpBody + 'b>> = None;

        if let Some(boxed) = process_downgrade!(init, Vec<u8>, http_body, backup) {
          boxed_err(req.send_body(*boxed).await)
        } else if let Some(boxed) = process_downgrade!(resume, String, backup) {
          boxed_err(req.send_body(*boxed).await)
        } else if let Some(boxed) = process_downgrade!(resume, &'static [u8], backup) {
          boxed_err(req.send_body(*boxed).await)
        } else {
          let error: Result<ClientResponse, Box<dyn Error>> =
            Err::<ClientResponse, Box<dyn Error>>(Box::new(BodyError()) as Box<dyn Error>);
          error
        }
      } else {
        boxed_err(req.send().await)
      }
    })
  }
}

fn boxed_err<R, E: Error + 'static>(result: Result<R, E>) -> Result<R, Box<dyn Error>> {
  result.map_err(|e| Box::new(e) as Box<dyn Error>)
}
