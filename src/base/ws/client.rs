use atomic::Atomic;
use bytemuck::{Pod, Zeroable};
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use ntex::channel::{mpsc, oneshot};
use ntex::tls::rustls::TlsConnector;
use ntex::util::{ByteString, Bytes};
use ntex::ws::WsSink;
use ntex::{channel::mpsc::Sender, rt, time, ws};
use ratelimit::Ratelimiter;
use rustls::RootCertStore;
use std::cell::RefCell;
use std::mem;
use std::ops::DerefMut;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::task::{Context, Poll};
use std::{error::Error, time::Duration};
use url::Url;

pub struct WsOptions {
  pub verbose: bool,
  pub ping_interval: Duration,
  pub backoff_base: Duration,
  pub max_backoff: Duration,
  pub send_limit: Option<Arc<Ratelimiter>>,
}

impl WsOptions {
  pub fn with_limiter(send_limit: Arc<Ratelimiter>) -> Self {
    Self {
      send_limit: Some(send_limit),
      ..Self::default()
    }
  }
}

impl Default for WsOptions {
  fn default() -> Self {
    Self {
      verbose: false,
      ping_interval: Duration::from_secs(30),
      backoff_base: Duration::from_millis(500),
      max_backoff: Duration::from_secs(30),
      send_limit: None,
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
    let connected = ConnectStatus::Connected as u8;
    let connecting = ConnectStatus::Connecting as u8;
    let disconnected = ConnectStatus::Disconnected as u8;

    match value.0 {
      x if x == disconnected => Ok(ConnectStatus::Disconnected),
      x if x == connecting => Ok(ConnectStatus::Connecting),
      x if x == connected => Ok(ConnectStatus::Connected),
      _ => Err(()),
    }
  }
}

struct ConditionalFuture<F: Future + Unpin> {
  inner_future: Option<F>,
}

impl<F: Future + Unpin> ConditionalFuture<F> {
  fn new(future: F) -> Self {
    ConditionalFuture {
      inner_future: Some(future),
    }
  }
}

impl<F: Future + Unpin> Future for ConditionalFuture<F> {
  type Output = Result<F::Output, F>;

  fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
    let mut this = self.as_mut();

    let mut inner_future = this.inner_future.take().unwrap();
    let pinned = Pin::new(&mut inner_future);

    match pinned.poll(cx) {
      Poll::Ready(val) => Poll::Ready(Ok(val)),
      Poll::Pending => Poll::Ready(Err(inner_future)),
    }
  }
}

macro_rules! project {
  ($self:ident) => {{
    ConditionalFuture::new($self).await
  }};
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

    //COMECA AQUI

    rt::spawn({
      let on_error = self.on_error.clone();
      let connect_status = self.connect_status.clone();
      let ping_interval = self.options.ping_interval;

      let on_binary = self.on_binary.clone();
      let on_message = self.on_message.clone();
      let on_close = self.on_close.clone();

      let limiter = self.options.send_limit.clone();

      async move {
        let seal = ws_client.seal();
        let sink = seal.sink();

        async fn send(
          sink: WsSink,
          limiter: Option<Arc<Ratelimiter>>,
          message: ws::Message,
        ) -> Result<(), ws::error::ProtocolError> {
          if let Some(send_limiter) = limiter {
            loop {
              match send_limiter.try_wait() {
                Ok(()) => {
                  return sink.send(message).await;
                }
                Err(duration) => {
                  ntex::time::sleep(duration).await;
                }
              }
            }
          } else {
            sink.send(message).await
          }
        }

        // Our struct client tasks
        let mut client_tasks = FuturesUnordered::new();

        //External message of sink.send
        let mut ext_tasks = FuturesUnordered::new();

        let mut ping_intev = time::interval(ping_interval);

        let mut rx_ws = seal.receiver();

        let error = loop {
          macros::select! {
            ping_res = ping_intev.next() => {
              if let None = ping_res {
                break format!("Internal error in ping");
              }

              ext_tasks.push(send(sink.clone(), limiter.clone(), ws::Message::Ping(Bytes::new())));
            },
            message = rx_ws.next() => match message {
              Some(result) => {
                match result {
                  Ok(ws::Frame::Binary(bin)) => {
                    let task = (on_binary)(bin.to_vec());
                    let task_projection = project!(task);

                    if task_projection.is_err() {
                      client_tasks.push(task_projection.err().unwrap());
                    }
                  }
                  Ok(ws::Frame::Text(txt)) => {
                    let str = String::from_utf8(txt.to_vec());
                    if let Ok(txt) = str {
                      let task = (on_message)(txt);
                      let task_projection = project!(task);

                      if task_projection.is_err() {
                        client_tasks.push(task_projection.err().unwrap());
                      }
                    } else {
                      break "Parser utf8 failed".to_string();
                    }
                  }
                  Ok(ws::Frame::Ping(p)) => {
                    ext_tasks.push(send(sink.clone(), limiter.clone(), ws::Message::Pong(p)));
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
            maybe_cmd = rx.recv() => match maybe_cmd {
              Some(cmd) => {
                ext_tasks.push(send(sink.clone(), limiter.clone(), cmd));
              }
              None => {
                break "Closed channel [maybe_cmd]".to_string();
              }
            },
            client_res = client_tasks.next(), if client_tasks.len() > 0 => {
              if let None = client_res {
                break "Wsclient failed".to_string();
              }
            }
            ext_res = ext_tasks.next(), if ext_tasks.len() > 0 => match ext_res {
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
