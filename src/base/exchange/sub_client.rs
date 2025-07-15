use crate::base::exchange::order::OrderBook;
use crate::base::ws::client::WsClient;
use crate::base::ws::client::WsOptions;
use ntex::channel::oneshot;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::pin::Pin;
use std::rc::Rc;

pub type Pending = Rc<RefCell<HashMap<String, Vec<oneshot::Sender<Rc<OrderBook>>>>>>;
pub type Subscribed = Rc<RefCell<HashMap<String, OrderBook>>>;

type DynString = Pin<Box<dyn Future<Output = Result<String, Box<dyn Error>>>>>;

pub struct SubClient {
  ws: WsClient,
  pub pending: Pending,
  pub subscribed: Subscribed,
  subscribe: Box<dyn Fn(String) -> DynString>,
  unsubscribe: Box<dyn Fn(String) -> DynString>,
}

#[derive(Clone)]
pub struct Shared {
  pub pending: Pending,
  pub subscribed: Subscribed,
}

impl SubClient {
  pub fn new<MessageRet, BinaryRet, SubscribeRet, UnsubscribeRet>(
    ws_url: &'static str,
    on_message: impl Fn(String, Shared) -> MessageRet + 'static,
    on_binary: impl Fn(Vec<u8>, Shared) -> BinaryRet + 'static,
    on_fail: impl Fn() + 'static,
    subscribe: impl Fn(String) -> SubscribeRet + 'static,
    unsubscribe: impl Fn(String) -> UnsubscribeRet + 'static,
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

    let shared = Shared {
      pending: pending.clone(),
      subscribed: subscribed.clone(),
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
      move |_| {
        Self::on_fail(s2.clone());
        fail1();
      },
      // on_close
      move || {
        Self::on_fail(s3);
        fail2();
      },
      // on_connected
      || {},
      move |binary| {
        let on_binary_cl = on_binary_rc.clone();
        let s4 = s4.clone();

        async move { on_binary_cl(binary, s4).await }
      },
      WsOptions::default(),
    );

    let subscribe_pin: Box<dyn Fn(String) -> DynString> =
      Box::new(move |symbol| Box::pin(subscribe(symbol)));

    let unsubscribe_pin: Box<dyn Fn(String) -> DynString> =
      Box::new(move |symbol| Box::pin(unsubscribe(symbol)));

    Self {
      ws,
      pending,
      subscribed,
      subscribe: subscribe_pin,
      unsubscribe: unsubscribe_pin,
    }
  }

  pub fn on_fail(shared: Shared) {
    shared.subscribed.borrow_mut().clear();
    shared.pending.borrow_mut().clear();
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
  pub async fn watch(&mut self, symbol: &str) -> Result<Rc<OrderBook>, Box<dyn Error>> {
    if !self.ws.is_connected() {
      self.connect().await?;
    }

    // prepara canal e detecta se é primeira vez
    let (tx, rx) = oneshot::channel();

    let mut first = false;
    {
      let mut subs = self.subscribed.borrow_mut();
      let mut map = self.pending.borrow_mut();
      if !subs.contains_key(symbol) {
        subs.insert(
          symbol.to_string(),
          OrderBook {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            update_id: 0,
          },
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
        self.ws.send(json).await?;
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
  pub async fn unwatch(&mut self, symbol: &str) -> Result<(), Box<dyn Error>> {
    let was_subscribed = self.subscribed.borrow_mut().remove(symbol);
    if was_subscribed.is_some() {
      let result = (self.unsubscribe)(symbol.to_string()).await;
      if let Ok(json) = result {
        self.pending.borrow_mut().remove(symbol);
        self.ws.send(json).await?
      } else {
        self
          .subscribed
          .borrow_mut()
          .insert(symbol.to_string(), was_subscribed.unwrap());
      }
    }
    Ok(())
  }

  pub async fn connect(&mut self) -> Result<(), Box<dyn Error>> {
    if !self.ws.is_connected() {
      self.ws.connect().await
    } else {
      Ok(())
    }
  }
}
