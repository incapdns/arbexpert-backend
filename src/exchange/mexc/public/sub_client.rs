use crate::base::exchange::order::OrderBook;
use crate::base::exchange::order::OrderBookUpdate;
use crate::base::ws::client::WsClient;
use crate::base::ws::client::WsOptions;
use crate::exchange::mexc::mexc_proto::PushDataV3ApiWrapper;
use crate::exchange::mexc::mexc_proto::push_data_v3_api_wrapper::Body;
use ntex::channel::oneshot;
use prost::Message;
use rust_decimal::Decimal;
use serde_json::json;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::mem;
use std::rc::Rc;

type Pending = Rc<RefCell<HashMap<String, Vec<oneshot::Sender<Rc<OrderBook>>>>>>;
type Subscribed = Rc<RefCell<HashMap<String, OrderBook>>>;

pub struct SubClient {
  ws: WsClient,
  pending: Pending,
  subscribed: Subscribed,
}

impl SubClient {
  pub fn handle_binary(data: Vec<u8>, pending: Pending, subscribed: Subscribed) -> Option<()> {
    let wrapper = PushDataV3ApiWrapper::decode(&*data).ok()?;

    if !wrapper
      .channel
      .contains("spot@public.increase.depth.batch.v3.api.pb@")
    {
      return None;
    }

    let symbol = wrapper.channel.rsplit('@').next().unwrap();
    let mut book = mem::take(subscribed.borrow_mut().get_mut(symbol)?);

    let old_id = book.update_id;

    match wrapper.body? {
      Body::PublicIncreaseDepthsBatch(data) => {
        for item in data.items {
          let update = OrderBookUpdate {
            update_id: 0,
            asks: item
              .asks
              .iter()
              .map(|item| {
                (
                  item.price.parse::<Decimal>().unwrap(),
                  item.quantity.parse::<Decimal>().unwrap(),
                )
              })
              .collect::<Vec<(Decimal, Decimal)>>(),
            bids: item
              .bids
              .iter()
              .map(|item| {
                (
                  item.price.parse::<Decimal>().unwrap(),
                  item.quantity.parse::<Decimal>().unwrap(),
                )
              })
              .collect::<Vec<(Decimal, Decimal)>>(),
          };

          book.apply_update(update);
        }
      }
      _ => {}
    }

    subscribed
      .borrow_mut()
      .insert(symbol.to_string(), book.clone());

    if old_id == book.update_id {
      return Some(());
    }

    let book = Rc::new(book);

    let subscriptions = mem::take(pending.borrow_mut().get_mut(symbol)?);

    for subscription in subscriptions {
      let _ = subscription.send(book.clone());
    }

    Some(())
  }

  pub fn on_fail(pending: Pending, subscribed: Subscribed) {
    subscribed.borrow_mut().clear();

    pending.borrow_mut().clear();
  }

  /// Cria e conecta imediatamente
  pub fn new(ws_url: &str) -> Self {
    let pending: Pending = Rc::new(RefCell::new(HashMap::new()));
    let subscribed: Subscribed = Rc::new(RefCell::new(HashMap::new()));
    let pending_c1 = pending.clone();
    let pending_c2 = pending.clone();
    let pending_c3 = pending.clone();
    let subscribed_c1 = subscribed.clone();
    let subscribed_c2 = subscribed.clone();
    let subscribed_c3 = subscribed.clone();

    let ws = WsClient::new(
      ws_url,
      // on_message
      move |_: String| async {},
      // on_error
      move |_| Self::on_fail(pending_c1.clone(), subscribed_c1.clone()),
      // on_close
      move || Self::on_fail(pending_c2.clone(), subscribed_c2.clone()),
      // on_connected
      || {},
      move |binary| {
        let pending_c3 = pending_c3.clone();
        let subscribed_c3= subscribed_c3.clone();

        async move {
          Self::handle_binary(binary, pending_c3.clone(), subscribed_c3.clone());
        }
      },
      WsOptions::default(),
    );

    SubClient {
      ws,
      pending,
      subscribed,
    }
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
      let msg = json!({
        "method": "SUBSCRIPTION",
        "params": [ format!("spot@public.increase.depth.batch.v3.api.pb@{}", symbol) ],
      })
      .to_string();
      self.ws.send(msg).await?;
    }

    // aguarda o próximo OrderBook
    let book = rx.await?;
    Ok(book)
  }

  /// Cancela inscrição e limpa pendentes
  pub async fn unwatch(&mut self, symbol: &str) -> Result<(), Box<dyn Error>> {
    let was_subscribed = self.subscribed.borrow_mut().remove(symbol);
    if was_subscribed.is_some() {
      self.pending.borrow_mut().remove(symbol);
      let msg = json!({
          "method": "UNSUBSCRIPTION",
          "params": [ format!("spot@public.increase.depth.batch.v3.api.pb@{}", symbol) ],
          "id": 1
      })
      .to_string();
      self.ws.send(msg).await?;
    }
    Ok(())
  }

  pub async fn connect(&mut self) -> Result<(), Box<dyn Error>> {
    self.ws.connect().await
  }
}