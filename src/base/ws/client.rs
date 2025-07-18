use futures::future::pending;
use futures::stream::FuturesUnordered;
use futures::{FutureExt, Stream, select};
use futures_util::StreamExt;
use ntex::channel::mpsc;
use ntex::tls::rustls::TlsConnector;
use ntex::util::ByteString;
use ntex::{channel::mpsc::Sender, rt, time, util::Bytes, ws};
use rustls::RootCertStore;
use std::cell::RefCell;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
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
      verbose: false,
      ping_interval: Duration::from_secs(30),
      backoff_base: Duration::from_millis(500),
      max_backoff: Duration::from_secs(30),
    }
  }
}

struct CallbackConditionalParams<P> {
  ready: Rc<AtomicBool>,
  param: Rc<RefCell<Option<P>>>,
}

impl<P> CallbackConditionalParams<P> {
  fn set_ready(&self, ready: bool) {
    self.ready.store(ready, Ordering::Relaxed);
  }

  fn set_param(&self, param: P) {
    let mut param_bm = self.param.borrow_mut();
    *param_bm = Some(param)
  }
}

struct CallbackConditional<P, F, R, Fut>
where
  F: Fn(Option<P>) -> Fut + Unpin,
  Fut: Future<Output = R> + Unpin,
{
  ready: Rc<AtomicBool>,
  param: Rc<RefCell<Option<P>>>,
  callback: F,
  future: Fut,
}
impl<P, F, R, Fut> CallbackConditional<P, F, R, Fut>
where
  F: Fn(Option<P>) -> Fut + Unpin,
  Fut: Future<Output = R> + Unpin,
{
  fn new(
    callback: F,
  ) -> (
    CallbackConditional<P, F, R, Fut>,
    CallbackConditionalParams<P>,
  ) {
    let ready = Rc::new(AtomicBool::new(false));
    let param = Rc::new(RefCell::new(None));

    (
      CallbackConditional {
        ready: ready.clone(),
        param: param.clone(),
        future: callback(None),
        callback,
      },
      CallbackConditionalParams {
        ready: ready.clone(),
        param: param.clone(),
      },
    )
  }
}

impl<P, F, R, Fut> Stream for CallbackConditional<P, F, R, Fut>
where
  F: Fn(Option<P>) -> Fut + Unpin,
  Fut: Future<Output = R> + Unpin,
{
  type Item = Fut::Output;

  fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    if !self.ready.load(Ordering::Relaxed) {
      return Poll::Pending;
    }

    let param = {
      let mut param_bm = self.param.borrow_mut();
      param_bm.take()
    };
    let this = self.get_mut();
    this.future = (this.callback)(param);
    let future = Pin::new(&mut this.future);
    match future.poll(cx) {
      Poll::Pending => Poll::Pending,
      Poll::Ready(res) => Poll::Ready(Some(res)),
    }
  }
}

pub struct WsClient {
  url: String,
  options: WsOptions,
  on_message: Rc<dyn Fn(String) -> Pin<Box<dyn Future<Output = ()>>>>,
  on_error: Rc<dyn Fn(String)>,
  on_close: Rc<dyn Fn()>,
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
    on_close: impl Fn() + 'static,
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
      on_close: Rc::from(on_close),
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
    if self.is_connected() {
      return Ok(());
    }

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

    async fn next_task<F: Future>(tasks: &mut FuturesUnordered<F>) -> Option<F::Output> {
      if tasks.len() > 0 {
        tasks.next().await
      } else {
        pending().await
      }
    }

    //COMECA AQUI

    let sink = ws_client.sink();
    rt::spawn({
      let on_error = self.on_error.clone();
      let is_connected = self.is_connected.clone();
      let ping_interval = self.options.ping_interval;

      let sink = sink.clone();
      let on_binary = self.on_binary.clone();
      let on_message = self.on_message.clone();
      let on_close = self.on_close.clone();

      async move {
        // Our struct client tasks
        let mut client_tasks = FuturesUnordered::new();

        //External message of sink.send
        let mut ext_tasks = FuturesUnordered::new();

        let mut ping_interval = time::interval(ping_interval);
        let mut next_ping_interval = ping_interval.next().fuse();

        let (mut ping, ping_config) = CallbackConditional::new(|_| {
          let sink = sink.clone();
          Box::pin(async move { sink.send(ws::Message::Ping(Bytes::new())).await })
        });
        let mut next_ping = ping.next().fuse();

        let (mut pong, pong_config) = CallbackConditional::new(|bytes| {
          let sink = sink.clone();

          Box::pin(async move {
            if let Some(bytes) = bytes {
              let result = sink.send(ws::Message::Pong(bytes)).await;
              if let Err(e) = result {
                return Err(e);
              }
            }

            Ok(())
          })
        });
        let mut next_pong = pong.next().fuse();

        let mut rx_ws = ws_client.seal().receiver();

        loop {
          select! {
            pong_res = next_pong => {
              if let None = pong_res {
                (on_error)(format!("Internal error in pong"));
                break;
              }

              if let Err(e) = pong_res.unwrap() {
                (on_error)(format!("Ping error: {}", e));
                break;
              }
            },
            maybe_cmd = rx.recv().fuse() => match maybe_cmd {
              Some(cmd) => {
                ext_tasks.push(sink.send(cmd));
              }
              None => {
                (on_error)("Closed channel".to_string());
                break;
              }
            },
            _ = next_ping_interval => {
              ping_config.set_ready(true);
              ping_config.set_param(());
            },
            ping_res = next_ping => {
              if let None = ping_res {
                (on_error)(format!("Internal error in ping"));
                break;
              }

              if let Err(e) = ping_res.unwrap() {
                (on_error)(format!("Ping error: {}", e));
                break;
              }

              ping_config.set_ready(false);
              next_ping = ping.next().fuse();
              next_ping_interval = ping_interval.next().fuse();
            },
            message = rx_ws.next().fuse() => match message {
              Some(result) => {
                match result {
                  Ok(ws::Frame::Binary(bin)) => {
                    client_tasks.push((on_binary)(bin.to_vec()));
                  }
                  Ok(ws::Frame::Text(txt)) => {
                    let str = String::from_utf8(txt.to_vec());
                    if let Ok(txt) = str {
                      client_tasks.push((on_message)(txt));
                    } else {
                      (on_error)("Parser utf8 failed".to_string());
                      break;
                    }
                  }
                  Ok(ws::Frame::Ping(p)) => {
                    pong_config.set_ready(true);
                    pong_config.set_param(p);
                    next_pong = pong.next().fuse();
                  }
                  Ok(ws::Frame::Close(e)) => {
                    (on_error)(format!("Closed socket: {:?}", e));
                    (on_close)();
                    break;
                  }
                  Err(e) => {
                    (on_error)(format!("WebSocket error: {:?}", e));
                    break;
                  }
                  _ => {
                    (on_error)(format!("Unknown error"));
                    break
                  }
                }
              }
              None => {
                (on_error)("Closed channel".to_string());
                break;
              }
            },
            client_res = next_task(&mut client_tasks).fuse() => {
              if let None = client_res {
                (on_error)("Wsclient failed".to_string());
                break;
              }
            }
            ext_res = next_task(&mut ext_tasks).fuse() => match ext_res {
              None => {
                (on_error)("Closed channel".to_string());
                break;
              },
              Some(result) => {
                if let Err(e) = result {
                  (on_error)(format!("Send external failed: {}", e));
                  break;
                }
              }
            }
          }
        }

        is_connected.store(false, Ordering::Relaxed);
      }
    });

    //SO ACIMA

    Ok(())
  }

  // Método para enviar mensagem texto ao servidor via canal
  pub fn send(&self, msg: String) -> Result<(), Box<dyn Error>> {
    if let Some(sender) = &self.sender {
      sender
        .send(ws::Message::Text(ByteString::from(msg)))
        .map_err(|e| e.into())
    } else {
      Err("WebSocket not connected".into())
    }
  }
}
