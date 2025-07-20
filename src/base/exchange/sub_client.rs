use crate::base::exchange::order::OrderBook;
use crate::base::ws::client::WsClient;
use crate::base::ws::client::WsOptions;
use ntex::channel::oneshot;
use ratelimit::Ratelimiter;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::mem;
use std::ops::DerefMut;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;

pub type SharedBook = Rc<RefCell<OrderBook>>;

pub type Pending = Rc<RefCell<HashMap<String, Vec<oneshot::Sender<SharedBook>>>>>;
pub type Subscribed = Rc<RefCell<HashMap<String, SharedBook>>>;

type DynString = Pin<Box<dyn Future<Output = Result<String, Box<dyn Error>>>>>;

pub struct SubClient {
  ws: RefCell<WsClient>,
  pub pending: Pending,
  pub subscribed: Subscribed,
  subscribe: Box<dyn Fn(String) -> DynString>,
  unsubscribe: Box<dyn Fn(String) -> DynString>,
  connecting: Rc<RefCell<Vec<oneshot::Sender<Result<(), Box<dyn Error>>>>>>,
  connect_limiter: Arc<Ratelimiter>
}

#[derive(Clone)]
pub struct Shared {
  pub pending: Pending,
  pub subscribed: Subscribed,
  connecting: Rc<RefCell<Vec<oneshot::Sender<Result<(), Box<dyn Error>>>>>>,
}

impl SubClient {
  pub fn new<MessageRet, BinaryRet, SubscribeRet, UnsubscribeRet>(
    ws_url: &'static str,
    on_message: impl Fn(String, Shared) -> MessageRet + 'static,
    on_binary: impl Fn(Vec<u8>, Shared) -> BinaryRet + 'static,
    on_fail: impl Fn() + 'static,
    subscribe: impl Fn(String) -> SubscribeRet + 'static,
    unsubscribe: impl Fn(String) -> UnsubscribeRet + 'static,
    connect_limiter: Arc<Ratelimiter>,
    send_limiter: Arc<Ratelimiter>,
  ) -> Self
  where
    MessageRet: Future<Output = ()> + 'static,
    BinaryRet: Future<Output = ()> + 'static,
    SubscribeRet: Future<Output = Result<String, Box<dyn Error>>> + 'static,
    UnsubscribeRet: Future<Output = Result<String, Box<dyn Error>>> + 'static,
  {
    let pending: Pending = Rc::new(RefCell::new(HashMap::new()));
    let subscribed: Subscribed = Rc::new(RefCell::new(HashMap::new()));

    let on_binary_rc = Rc::new(on_binary);
    let on_message_rc = Rc::new(on_message);

    let connecting = Rc::new(RefCell::new(vec![]));

    let shared = Shared {
      pending: pending.clone(),
      subscribed: subscribed.clone(),
      connecting: connecting.clone(),
    };

    let on_fail = Rc::new(on_fail);

    let (fail1, fail2) = (on_fail.clone(), on_fail);

    let (s1, s2, s3, s4) = (shared.clone(), shared.clone(), shared.clone(), shared);

    let ws = WsClient::new(
      ws_url,
      // on_message
      move |message| {
        let on_message_cl = on_message_rc.clone();
        let s1 = s1.clone();

        async move { on_message_cl(message, s1).await }
      },
      // on_error
      move |err| {
        println!("{ws_url}: {err}");
        Self::on_fail(s2.clone());
        fail1();
      },
      // on_close
      move || {
        Self::on_fail(s3.clone());
        fail2();
      },
      // on_connected
      move || {},
      move |binary| {
        let on_binary_cl = on_binary_rc.clone();
        let s4 = s4.clone();

        async move { on_binary_cl(binary, s4).await }
      },
      WsOptions::with_limit(send_limiter),
    );

    let subscribe_pin: Box<dyn Fn(String) -> DynString> =
      Box::new(move |symbol| Box::pin(subscribe(symbol)));

    let unsubscribe_pin: Box<dyn Fn(String) -> DynString> =
      Box::new(move |symbol| Box::pin(unsubscribe(symbol)));

    Self {
      ws: RefCell::new(ws),
      pending,
      subscribed,
      subscribe: subscribe_pin,
      unsubscribe: unsubscribe_pin,
      connecting,
      connect_limiter
    }
  }

  pub fn on_fail(shared: Shared) {
    shared.subscribed.borrow_mut().clear();
    shared.pending.borrow_mut().clear();
    shared.connecting.borrow_mut().clear();
  }

  /// Diz se este subcliente já tem `symbol`
  pub fn has_symbol(&self, symbol: &str) -> bool {
    self.subscribed.borrow().contains_key(symbol)
  }

  /// Quantos canais já foram inscritos
  pub fn subscribed_count(&self) -> usize {
    self.subscribed.borrow().len()
  }

  /// Pega o _próximo_ OrderBook para `symbol`. Se for a primeira chamada, envia SUBSCRIBE.
  pub async fn watch(&self, symbol: &str) -> Result<SharedBook, Box<dyn Error>> {
    self.connect().await?;

    // prepara canal e detecta se é primeira vez
    let (tx, rx) = oneshot::channel();

    let mut first = false;
    {
      let mut subs = self.subscribed.borrow_mut();
      let mut map = self.pending.borrow_mut();
      if !subs.contains_key(symbol) {
        subs.insert(
          symbol.to_string(),
          Rc::new(RefCell::new(OrderBook {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            update_id: 0,
          })),
        );
        map.entry(symbol.to_string()).or_insert_with(|| Vec::new());
        first = true;
      }
      map.get_mut(symbol).unwrap().push(tx);
    }

    // apenas no primeiro watch: subscribe
    if first {
      let result = (self.subscribe)(symbol.to_string()).await;
      if let Ok(json) = result {
        self.send(json).await?
      } else {
        self.subscribed.borrow_mut().remove(symbol);
        self.pending.borrow_mut().remove(symbol);

        return Err("Invalid json".into());
      }
    }

    // aguarda o próximo OrderBook
    let book = rx.await?;
    Ok(book)
  }

  /// Cancela inscrição e limpa pendentes
  pub async fn unwatch(&self, symbol: &str) -> Result<(), Box<dyn Error>> {
    let was_subscribed = self.subscribed.borrow_mut().remove(symbol);
    if was_subscribed.is_some() {
      let result = (self.unsubscribe)(symbol.to_string()).await;
      if let Ok(json) = result {
        self.pending.borrow_mut().remove(symbol);
        self.send(json).await?
      } else {
        self
          .subscribed
          .borrow_mut()
          .insert(symbol.to_string(), was_subscribed.unwrap());
      }
    }
    Ok(())
  }

  async fn send(&self, json: String) -> Result<(), Box<dyn Error>> {
    let ws_bm = self.ws.try_borrow_mut();
    ws_bm?.send(json)?;
    return Ok(());
  }

  async fn connect(&self) -> Result<(), Box<dyn Error>> {
    let mut connecting = None;

    {
      let ws_bm = self.ws.try_borrow_mut();

      if let Ok(mut ws) = ws_bm {
        if ws.is_connected() {
          return Ok(());
        }

        let (result, senders) = loop {
          match self.connect_limiter.try_wait() {
            Ok(()) => {
              let result = ws.connect().await;
              break (result, {
                let mut connecting_bm = self.connecting.borrow_mut();
                mem::take(connecting_bm.deref_mut())
              });
            }
            Err(duration) => {
              ntex::time::sleep(duration).await;
            }
          }
        };

        if result.is_ok() {
          connecting = Some(senders);
        }
      } else {
        let (tx, rx) = oneshot::channel();
        {
          let mut connecting_bm = self.connecting.borrow_mut();
          connecting_bm.push(tx);
        }
        let _ = rx.await?;
      }
    }

    if let Some(senders) = connecting {
      for sender in senders {
        let _ = sender.send(Ok(()));
      }
    }

    Ok(())
  }
}
