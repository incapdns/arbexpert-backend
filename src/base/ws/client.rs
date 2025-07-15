use futures::future::pending;
use futures::stream::FuturesUnordered;
use futures::{FutureExt, select};
use futures_util::StreamExt;
use ntex::channel::mpsc;
use ntex::tls::rustls::TlsConnector;
use ntex::util::ByteString;
use ntex::{channel::mpsc::Sender, rt, time, util::Bytes, ws};
use rustls::RootCertStore;
use serde_json::Value;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{error::Error, time::Duration};
use url::Url;

#[derive(Clone)]
pub struct WsOptions {
  pub verbose: bool,
  pub ping_interval: Duration,
  pub backoff_base: Duration,
  pub max_backoff: Duration,
}

impl Default for WsOptions {
  fn default() -> Self {
    Self {
      verbose: true,
      ping_interval: Duration::from_secs(30),
      backoff_base: Duration::from_millis(500),
      max_backoff: Duration::from_secs(30),
    }
  }
}

pub struct WsClient {
  url: String,
  options: WsOptions,
  on_message: Rc<dyn Fn(String) -> Pin<Box<dyn Future<Output = ()>>>>,
  on_error: Rc<dyn Fn(String)>,
  on_close: Option<Box<dyn FnOnce()>>,
  on_connected: Rc<dyn Fn()>,
  on_binary: Rc<dyn Fn(Vec<u8>) -> Pin<Box<dyn Future<Output = ()>>>>,
  sender: Option<Sender<ws::Message>>,
  is_connected: Rc<AtomicBool>,
}

impl WsClient {
  pub fn new<OnMessageRet, OnBinaryRet>(
    url: impl Into<String>,
    on_message: impl Fn(String) -> OnMessageRet + 'static,
    on_error: impl Fn(String) + 'static,
    on_close: impl FnOnce() + 'static,
    on_connected: impl Fn() + 'static,
    on_binary: impl Fn(Vec<u8>) -> OnBinaryRet + 'static,
    options: WsOptions,
  ) -> Self
  where
    OnMessageRet: Future<Output = ()> + 'static,
    OnBinaryRet: Future<Output = ()> + 'static,
  {
    let on_message_pin: Rc<dyn Fn(String) -> Pin<Box<dyn futures::Future<Output = ()>>>> =
      Rc::new(move |message| Box::pin(on_message(message)));

    let on_binary_pin: Rc<dyn Fn(Vec<u8>) -> Pin<Box<dyn futures::Future<Output = ()>>>> =
      Rc::new(move |message| Box::pin(on_binary(message)));

    Self {
      url: url.into(),
      options,
      on_message: on_message_pin,
      on_error: Rc::from(on_error),
      on_close: Some(Box::new(on_close)),
      on_connected: Rc::from(on_connected),
      on_binary: on_binary_pin,
      sender: None,
      is_connected: Rc::new(AtomicBool::new(false)),
    }
  }

  pub fn is_connected(&self) -> bool {
    self.is_connected.load(Ordering::Relaxed)
  }

  pub async fn close(&self) -> Result<(), Box<dyn Error>> {
    if let Some(sender) = &self.sender {
      sender.send(ws::Message::Close(None)).map_err(|e| e.into())
    } else {
      Err("WebSocket not connected".into())
    }
  }

  pub async fn connect(&mut self) -> Result<(), Box<dyn Error>> {
    let url = Url::parse(&self.url)?;
    if self.options.verbose {
      println!("Connecting to {}", url);
    }

    let connector = {
      let root_store = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.into(),
      };

      let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

      TlsConnector::new(config)
    };

    let ws_client = ws::WsClient::build(&self.url)
      .connector(connector)
      .finish()
      .map_err(|e| format!("Build error: {:?}", e))?
      .connect()
      .await
      .map_err(|e| format!("Connect error: {:?}", e))?;

    self.is_connected.store(true, Ordering::Relaxed);

    (self.on_connected)();

    let (tx, rx) = mpsc::channel::<ws::Message>();
    self.sender = Some(tx.clone());

    // Tarefa para enviar mensagens da fila (tx)
    let sink = ws_client.sink();
    rt::spawn(async move {
      while let Some(cmd) = rx.recv().await {
        let _ = sink.send(cmd).await;
      }
    });

    let (ic1, ic2) = (self.is_connected.clone(), self.is_connected.clone());

    let ping_sink = ws_client.sink();
    let ping_interval = self.options.ping_interval;
    let on_error = self.on_error.clone();
    rt::spawn(async move {
      loop {
        time::sleep(ping_interval).await;
        if ping_sink
          .send(ws::Message::Ping(Bytes::new()))
          .await
          .is_err()
        {
          ic1.store(false, Ordering::Relaxed);
          (on_error)("Ping error".to_string());
          break;
        }
      }
    });

    let (message_tx, message_rx) = mpsc::channel();
    let (binary_tx, binary_rx) = mpsc::channel();

    rt::spawn(async move {
      let mut tasks = FuturesUnordered::new();

      async fn next_task<F: Future>(tasks: &mut FuturesUnordered<F>) -> Option<F::Output> {
        if tasks.len() > 0 {
          tasks.next().await
        } else {
          pending().await
        }
      }

      loop {
        select! {
          opt = message_rx.recv().fuse() => {
            if let Some(future) = opt {
              tasks.push(future);
            } else {
              break;
            }
          },
          opt = binary_rx.recv().fuse() => {
            if let Some(future) = opt {
              tasks.push(future);
            } else {
              break;
            }
          },
          _ = next_task(&mut tasks).fuse() => {},
        };
      }
    });

    let sink = ws_client.sink();
    let mut rx_ws = ws_client.seal().receiver();

    let on_error = self.on_error.clone();
    let on_message = self.on_message.clone();
    let on_close = self.on_close.take().unwrap();
    let on_binary = self.on_binary.clone();
    rt::spawn(async move {
      while let Some(frame) = rx_ws.next().await {
        match frame {
          Ok(ws::Frame::Binary(bin)) => {
            let _ = binary_tx.send((on_binary)(bin.to_vec()));
          }
          Ok(ws::Frame::Text(txt)) => {
            let message = String::from_utf8(txt.to_vec());
            if let Ok(str) = message {
              let _ = message_tx.send((on_message)(str));
            }
          }
          Ok(ws::Frame::Ping(p)) => {
            let _ = sink.send(ws::Message::Pong(p)).await;
          }
          Ok(ws::Frame::Close(_)) => {
            ic2.store(false, Ordering::Relaxed);
            (on_close)();
            break;
          }
          Err(e) => {
            ic2.store(false, Ordering::Relaxed);
            (on_error)(format!("WebSocket error: {:?}", e));
            break;
          }
          _ => {}
        }
      }
    });

    Ok(())
  }

  // Método para enviar mensagem texto ao servidor via canal
  pub async fn send(&self, msg: String) -> Result<(), Box<dyn Error>> {
    if let Some(sender) = &self.sender {
      sender
        .send(ws::Message::Text(ByteString::from(msg)))
        .map_err(|e| e.into())
    } else {
      Err("WebSocket not connected".into())
    }
  }
}
