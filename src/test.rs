use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use crate::{
  base::{
    exchange::{order::OrderBook, sub_client::SharedBook, Exchange},
    http::{builder::HttpRequestBuilder, client::ntex::NtexHttpClient},
    ws::client::{WsClient, WsOptions},
  },
  exchange::{mexc::MexcExchange},
};
use compio::buf::bytes::Buf;
use futures::{FutureExt, StreamExt, stream::FuturesUnordered};
use ntex::{channel::mpsc, rt};
use std::{collections::BTreeMap, error::Error, time::Duration};

pub async fn do_websocket_test() -> Result<(), Box<dyn std::error::Error>> {
  let url = "wss://echo.websocket.events".to_string();

  let on_message = Box::new(|msg: String| async move {
    println!("Received message: {}", msg);
  });

  let on_error = Box::new(|err: String| {
    eprintln!("WebSocket error: {}", err);
  });

  let on_close = Box::new(|| {
    println!("WebSocket closed.");
  });

  let on_connected = Box::new(|| {
    println!("WebSocket connected.");
  });

  let on_binary = Box::new(|_| async {
    println!("Received binary.");
  });

  let options = WsOptions {
    verbose: true,
    ..Default::default()
  };

  let mut client = WsClient::new(
    url,
    on_message,
    on_error,
    on_close,
    on_connected,
    on_binary,
    options,
  );

  client.connect().await?;

  client
    .send("Hello from Glommio WebSocket client!".into());

  compio::time::sleep(Duration::from_millis(1)).await;

  // Fecha conexão
  if let Err(e) = client.close().await {
    eprintln!("Failed to close connection: {}", e);
  }

  Ok(())
}

pub async fn do_http_post_test() -> Result<(), Box<dyn std::error::Error>> {
  let method = "GET";
  let uri = "http://2.17.162.10/";

  let request_builder = HttpRequestBuilder::new(NtexHttpClient::new(), method, uri);

  let mut result = request_builder
    .set_header("From", "Glommio")
    .set_body("Eita Baby".to_string())
    .do_request()
    .await?;

  let bytes = result.body().await?;

  let bytes = String::from_utf8_lossy(bytes.chunk());

  println!("result: {:?}", bytes);

  Ok(())
}

pub async fn do_orderbook_test(
  exchange: &dyn Exchange,
) -> (Result<SharedBook, Box<dyn Error>>, Result<SharedBook, Box<dyn Error>>) {
  (
    exchange.watch_orderbook("BTC/USDT".to_string()).await,
    exchange.watch_orderbook("BTC/USDT:USDT".to_string()).await
  )
}

pub async fn do_watch_my_trades_test() {
  let mut exchange = MexcExchange::with_credentials(
    "mx0vgl85yzq1OILekB".to_string(),
    "b7d2664039284cb2aa911ea3fa38b9f0".to_string(),
    "WEBd640ba2683815e4fcae30ac8e774e584842bc843fe8d5aec35a25f26161e2c38".to_string(),
  )
  .await;

  let trades = exchange
    .fetch_last_orders("BOXCAT/USDT", Duration::from_secs(578400))
    .await;

  let catch_orders1 = exchange.catch_orders("VANRY/USDT").await;
  let catch_orders2 = exchange.catch_orders("VANRY/USDT").await;

  if let Err(err) = catch_orders1 {
    eprintln!("err 1: {:?}", err);
    return;
  }

  if let Err(err) = catch_orders2 {
    eprintln!("err 2: {:?}", err);
    return;
  }

  let catch_orders1 = catch_orders1.unwrap();
  let catch_orders2 = catch_orders2.unwrap();

  let next1 = catch_orders1.next(0).await;
  let next2 = catch_orders2.next(1).await;

  let (result1, result2) = (next1, next2);

  println!("Result 1: {:?}\nResult2: {:?}", result1, result2);
}

pub async fn sleep(duration: Duration) -> String {
  ntex::time::sleep(duration).await;
  format!("{}s", duration.as_secs())
}

pub async fn random_test() -> Result<(), Box<dyn Error>> {
  let (tx, rx) = mpsc::channel();

  rt::spawn(async move {
    let mut tasks = FuturesUnordered::new();

    loop {
      macros::select! {
        opt = rx.recv() => {
          println!("Chegou {}", opt.is_some());
          if let Some(future) = opt {
            tasks.push(future);
          } else {
            break;
          }
        },
        res = tasks.next(), if tasks.len() > 0  => {
          println!("{}, {:?}", tasks.len(), res);
        },
      };
    }
  });

  let _ = tx.send(sleep(Duration::from_secs(5)));

  sleep(Duration::from_millis(100)).await;

  let _ = tx.send(sleep(Duration::from_secs(1)));

  sleep(Duration::from_secs(6)).await;

  Ok(())
}

pub fn do_main_test() -> Option<()>{
  let mut map = Arc::new(HashMap::<i32, OrderBook>::new());

  let map_mut = Arc::get_mut(&mut map)?;

  map_mut.insert(1,  OrderBook {
    asks: BTreeMap::new(),
    bids: BTreeMap::new(),
    update_id: 100
  });

  let handle = thread::spawn({
    let cloned = map.clone();
    move || {
      // acesso seguro
      if let Some(res) = cloned.get(&1) {
        println!("Res: {:?}", res);
      }
    }
  });

  handle.join().unwrap();

  Some(())
}