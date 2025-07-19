use atomic::Atomic;
use bytemuck::{Pod, Zeroable};
use futures::future::pending;
use futures::stream::FuturesUnordered;
use futures::{FutureExt, Stream, select};
use futures_util::StreamExt;
use ntex::channel::{mpsc, oneshot};
use ntex::tls::rustls::TlsConnector;
use ntex::util::ByteString;
use ntex::{channel::mpsc::Sender, rt, time, util::Bytes, ws};
use rustls::RootCertStore;
use std::cell::RefCell;
use std::mem;
use std::ops::DerefMut;
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

#[repr(u8)]
#[derive(PartialEq)]
enum ConnectStatus {
  Connecting = 0,
  Connected = 1,
  Disconnected = 2,
}

#[repr(transparent)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, Pod, Zeroable)]
pub struct ConnectStatusRepr(pub u8);

pub struct WsClient {
  url: String,
  options: WsOptions,
  on_message: Rc<dyn Fn(String) -> Pin<Box<dyn Future<Output = ()>>>>,
  on_error: Rc<dyn Fn(String)>,
  on_close: Rc<dyn Fn()>,
  on_connected: Rc<dyn Fn()>,
  on_binary: Rc<dyn Fn(Vec<u8>) -> Pin<Box<dyn Future<Output = ()>>>>,
  sender: Option<Sender<ws::Message>>,
  connect_status: Rc<Atomic<ConnectStatusRepr>>,
  connecting: RefCell<Vec<oneshot::Sender<()>>>,
}

impl From<ConnectStatus> for ConnectStatusRepr {
  fn from(status: ConnectStatus) -> Self {
    ConnectStatusRepr(status as u8)
  }
}

impl TryFrom<ConnectStatusRepr> for ConnectStatus {
  type Error = ();

  fn try_from(value: ConnectStatusRepr) -> Result<Self, Self::Error> {
    let connected=  ConnectStatus::Connected as u8;
    let connecting=  ConnectStatus::Connecting as u8;
    let disconnected=  ConnectStatus::Disconnected as u8;

    match value.0 {
      x if x == disconnected => Ok(ConnectStatus::Disconnected),
      x if x == connecting => Ok(ConnectStatus::Connecting),
      x if x == connected => Ok(ConnectStatus::Connected),
      _ => Err(()),
    }
  }
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
      connect_status: Rc::new(Atomic::new(ConnectStatusRepr(
        ConnectStatus::Disconnected as u8,
      ))),
      connecting: RefCell::new(vec![]),
    }
  }

  pub fn is_connected(&self) -> bool {
    self.connect_status.load(Ordering::Relaxed).try_into() == Ok(ConnectStatus::Connected)
  }

  pub fn is_connecting(&self) -> bool {
    self.connect_status.load(Ordering::Relaxed).try_into() == Ok(ConnectStatus::Connecting)
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

    if self.is_connecting() {
      let rx = {
        let (tx, rx) = oneshot::channel();
        let mut connecting_bm = self.connecting.borrow_mut();
        connecting_bm.push(tx);
        rx
      };
      let result = rx.await;
      let result = result.map_err(|_| not_connected().err().unwrap());
      return result;
    } else {
      self.connect_status.store(
        ConnectStatusRepr(ConnectStatus::Connecting as u8),
        Ordering::Relaxed,
      );
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
      .map_err(|e| format!("Connect error: {:?}", e));

    let Ok(ws_client) = ws_client else {
      self.connect_status.store(
        ConnectStatusRepr(ConnectStatus::Disconnected as u8),
        Ordering::Relaxed,
      );
      let mut connecting_bm = self.connecting.borrow_mut();
      connecting_bm.clear();
      return not_connected();
    };

    let (tx, rx) = mpsc::channel::<ws::Message>();
    self.sender = Some(tx.clone());

    self.connect_status.store(
      ConnectStatusRepr(ConnectStatus::Connected as u8),
      Ordering::Relaxed,
    );

    (self.on_connected)();

    {
      let mut connecting_bm = self.connecting.borrow_mut();
      let connecting = mem::take(connecting_bm.deref_mut());
      for item in connecting {
        let _ = item.send(());
      }
    }

    async fn next_task<F: Future>(tasks: &mut FuturesUnordered<F>) -> Option<F::Output> {
      if tasks.len() > 0 {
        tasks.next().await
      } else {
        pending().await
      }
    }

    //COMECA AQUI

    rt::spawn({
      let on_error = self.on_error.clone();
      let connect_status = self.connect_status.clone();
      let ping_interval = self.options.ping_interval;

      let on_binary = self.on_binary.clone();
      let on_message = self.on_message.clone();
      let on_close = self.on_close.clone();

      async move {
        let seal = ws_client.seal();
        let sink = seal.sink();

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

        let mut rx_ws = seal.receiver();

        let error = loop {
          select! {
            ping_res = next_ping => {
              if let None = ping_res {
                break format!("Internal error in ping");
              }

              if let Err(e) = ping_res.unwrap() {
                break format!("Ping error: {}", e);
              }

              ping_config.set_ready(false);
              next_ping = ping.next().fuse();
              next_ping_interval = ping_interval.next().fuse();
            },
            pong_res = next_pong => {
              if let None = pong_res {
                break format!("Internal error in pong");
              }

              if let Err(e) = pong_res.unwrap() {
                break format!("Ping error: {}", e);
              }
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
                      break "Parser utf8 failed".to_string();
                    }
                  }
                  Ok(ws::Frame::Ping(p)) => {
                    pong_config.set_ready(true);
                    pong_config.set_param(p);
                    next_pong = pong.next().fuse();
                  }
                  Ok(ws::Frame::Close(e)) => {
                    break format!("Closed socket: {:?}", e);
                  }
                  Ok(ntex::ws::Frame::Continuation(_)) => {
                    break format!("Unsupported continuation");
                  }
                  Ok(ntex::ws::Frame::Pong(_)) => {}
                  Err(e) => {
                    break format!("WebSocket error: {:?}", e);
                  }
                }
              }
              None => {
                break "Closed channel [message]".to_string();
              }
            },
            maybe_cmd = rx.recv().fuse() => match maybe_cmd {
              Some(cmd) => {
                ext_tasks.push(sink.send(cmd));
              }
              None => {
                break "Closed channel [maybe_cmd]".to_string();
              }
            },
            _ = next_ping_interval => {
              ping_config.set_ready(true);
              ping_config.set_param(());
            },
            client_res = next_task(&mut client_tasks).fuse() => {
              if let None = client_res {
                break "Wsclient failed".to_string();
              }
            }
            ext_res = next_task(&mut ext_tasks).fuse() => match ext_res {
              None => {
                break "Closed channel [ext_res]".to_string();
              },
              Some(result) => {
                if let Err(e) = result {
                  break format!("Send external failed: {}", e);
                }
              }
            }
          }
        };

        connect_status.store(
          ConnectStatusRepr(ConnectStatus::Disconnected as u8),
          Ordering::Relaxed,
        );

        on_error(error);
        on_close();
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

fn not_connected() -> Result<(), Box<dyn Error>> {
  Err("Not connected".into())
}
