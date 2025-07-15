use downcast_rs::{Downcast, impl_downcast};
use std::pin::Pin;

pub trait HttpBody: Downcast {}

impl HttpBody for Vec<u8> {}
impl HttpBody for String {}
impl HttpBody for &'static [u8] {}

impl_downcast!(HttpBody);

pub type DynamicIterator<'a, T> = &'a mut dyn Iterator<Item = T>;

pub trait HttpClient {
  type Response;

  fn request<'t, 'b>(
    &'t self,
    method: String,
    uri: String,
    headers: DynamicIterator<'t, (&'t dyn AsRef<str>, &'t dyn AsRef<str>)>,
    body: Option<Box<dyn HttpBody + 'b>>,
  ) -> Pin<Box<dyn Future<Output = Result<Self::Response, Box<dyn std::error::Error>>> + 't>>;
}

#[macro_export]
macro_rules! from_headers {
  ($headers:expr) => {
    &mut $headers
      .iter()
      .map(|(k, v)| (k as &dyn AsRef<str>, v as &dyn AsRef<str>))
    as DynamicIterator<(&dyn AsRef<str>, &dyn AsRef<str>)>
  };
}

#[macro_export]
macro_rules! define_a {
  ($expr:expr) => {
    let mut a = $expr;
  };
}