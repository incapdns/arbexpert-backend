use crate::{
  base::exchange::assets::{Asset, Assets, MarketType},
  exchange::{binance::BinanceExchange, mexc::MexcExchange},
  worker::{
    commands::{Request, StartArbitrage, StartMonitor},
    state::GlobalState,
    worker_loop,
  },
};
use async_channel::unbounded;
use futures::{TryStreamExt, join};
use ntex::{
  Service, fn_service,
  http::HttpService,
  rt,
  server::Server,
  service::fn_factory_with_config,
  util::BytesMut,
  web::{self, App, HttpRequest, HttpResponse, middleware},
};
use rust_decimal::{Decimal, dec};
use rustls::crypto::aws_lc_rs::default_provider;
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
  cell::UnsafeCell, collections::{BTreeMap, HashMap}, net::SocketAddr, sync::{
    atomic::{AtomicU32, Ordering}, Arc, Mutex
  }, time::Duration, vec
};
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

pub mod arbitrage;
pub mod base;
pub mod exchange;
pub mod test;
pub mod worker;

#[web::post("/arbitrage/{symbol}/start")]
async fn start_arbitrage(
  _: HttpRequest,
  symbol: web::types::Path<String>,
  mut payload: web::types::Payload,
  data: web::types::State<Arc<GlobalState>>,
) -> HttpResponse {
  let mut body = BytesMut::new();

  while let Ok(chunk) = payload.try_next().await {
    let Some(chunk) = chunk else { continue };
    body.extend(chunk);
  }

  let parsed: serde_json::Result<StartArbitrage> = serde_json::from_slice(&body);

  let Ok(command) = parsed else {
    return HttpResponse::InternalServerError().body("Worker not found");
  };

  let symbol = symbol.into_inner().replace("-", "/");

  let worker_id = {
    let mut map = data.symbol_map.lock().unwrap();
    if let Some(&id) = map.get(&symbol) {
      id
    } else {
      let id = data.next_worker.fetch_add(1, Ordering::Relaxed);
      map.insert(symbol.clone(), id);
      id
    }
  };

  let tx = {
    let channels = data.worker_channels.lock().unwrap();
    channels.get(&worker_id).cloned()
  };

  if let Some(tx) = tx {
    let _ = tx.send(Request::StartArbitrage(command)).await;

    HttpResponse::Ok().body(format!("Started!"))
  } else {
    HttpResponse::InternalServerError().body("Worker not found")
  }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ArbitrageResult {
  pub entry_percent: Decimal,
  pub exit_percent: Decimal,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Arbitrage {
  pub spot: Asset,   // Spot de uma exchange
  pub future: Asset, // Futuro de outra exchange
  #[serde(with = "unsafe_cell_abr")]
  pub result: UnsafeCell<ArbitrageResult>,
}

mod unsafe_cell_abr {
  use serde::{Deserialize, Deserializer, Serialize, Serializer};
  use std::cell::UnsafeCell;

  use crate::ArbitrageResult;

  pub fn serialize<S>(cell: &UnsafeCell<ArbitrageResult>, serializer: S) -> Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    let inner = unsafe { &*cell.get() };
    inner.serialize(serializer)
  }

  pub fn deserialize<'de, D>(deserializer: D) -> Result<UnsafeCell<ArbitrageResult>, D::Error>
  where
    D: Deserializer<'de>,
  {
    let val = ArbitrageResult::deserialize(deserializer)?;
    Ok(UnsafeCell::new(val))
  }
}

async fn setup_exchanges() -> Vec<BinanceExchange> {
  let (a,) = join!(BinanceExchange::new(),);

  vec![a]
}

async fn cross_assets_all_exchanges(data: web::types::State<Arc<GlobalState>>) {
  let mut arbitrages = HashMap::new();

  // Instancia todas as exchanges
  let exchanges = setup_exchanges().await;

  // Mapa auxiliar: symbol → [(spot/future asset, tipo mercado, exchange name)]
  let mut symbol_map: BTreeMap<String, Vec<(&Asset, MarketType, String)>> = BTreeMap::new();

  for exchange in &exchanges {
    let exchange_name = exchange.name(); // ou `exchange.to_string()` dependendo da trait

    let assets = exchange.public.assets.as_ref().unwrap();

    // Prepara assets spot
    for asset in assets.spot.values() {
      symbol_map.entry(asset.symbol.clone()).or_default().push((
        asset,
        MarketType::Spot,
        exchange_name.clone(),
      ));
    }

    // Prepara assets futuro
    for asset in assets.future.values() {
      symbol_map.entry(asset.symbol.clone()).or_default().push((
        asset,
        MarketType::Future,
        exchange_name.clone(),
      ));
    }
  }

  // Agora cruza todos os pares possíveis por símbolo
  for entries in symbol_map.values() {
    let spot_assets: Vec<_> = entries
      .iter()
      .filter(|(_, market, _)| *market == MarketType::Spot)
      .collect();

    let future_assets: Vec<_> = entries
      .iter()
      .filter(|(_, market, _)| *market == MarketType::Future)
      .collect();

    for (spot, _, _) in &spot_assets {
      for (future, _, _) in &future_assets {
        let vec = arbitrages
          .entry(spot.symbol.clone())
          .or_insert(Arc::new(vec![]));

        let vec_mut = Arc::get_mut(vec).unwrap();

        vec_mut.push(Arc::new(Arbitrage {
          spot: (*spot).clone(),
          future: (*future).clone(),
          result: UnsafeCell::new(ArbitrageResult {
            entry_percent: dec!(0),
            exit_percent: dec!(0),
          }),
        }));
      }
    }
  }

  let mut i = 0;

  for (key, items) in &arbitrages {
    let worker_id = {
      let mut map = data.symbol_map.lock().unwrap();
      if let Some(&id) = map.get(key) {
        id
      } else {
        let id = data.next_worker.fetch_add(1, Ordering::Relaxed) % num_cpus::get() as u32;
        map.insert(key.clone(), id);
        id
      }
    };

    let tx = {
      let channels = data.worker_channels.lock().unwrap();
      channels.get(&worker_id).cloned()
    };

    if let Some(tx) = tx {
      let _ = tx.send(Request::StartMonitor(StartMonitor {
        items: items.clone()
      })).await;
    }

    if i % 7 == 0 {
      ntex::time::sleep(Duration::from_secs(2)).await;
    }

    i += 1;
  }
}

unsafe impl Sync for Arbitrage {}

#[web::post("/monitor/start")]
async fn start_monitor(data: web::types::State<Arc<GlobalState>>) -> HttpResponse {
  rt::spawn(cross_assets_all_exchanges(data));

  HttpResponse::Ok().body(format!("Starting"))
}

async fn index_async(_: HttpRequest) -> &'static str {
  "Hello world!\r\n"
}

#[web::get("/")]
async fn no_params() -> &'static str {
  "Hello world!\r\n"
}

#[web::get("/symbols")]
async fn symbols() -> String {
  let mexc = MexcExchange::new().await;
  let symbols: HashMap<String, exchange::mexc::public::symbols::SymbolEntry> = mexc.get_symbols();
  serde_json::to_string(&symbols).unwrap()
}

async fn on_worker_start(global_state: Arc<GlobalState>) -> Result<(), String> {
  let worker_id = global_state.last_id.fetch_add(1, Ordering::Relaxed);

  let (tx, rx) = unbounded();

  global_state
    .worker_channels
    .lock()
    .unwrap()
    .insert(worker_id, tx);

  ntex::rt::spawn(worker_loop(worker_id, rx, global_state));

  Ok(())
}

async fn ws_service(
  _: web::ws::WsSink,
) -> Result<
  impl Service<web::ws::Frame, Response = Option<web::ws::Message>, Error = std::io::Error>,
  web::Error,
> {
  let service = fn_service(move |frame| {
    let response = match frame {
      web::ws::Frame::Text(text) => Some(web::ws::Message::Text(
        String::from_utf8_lossy(&text).to_string().into(),
      )),
      web::ws::Frame::Binary(bin) => Some(web::ws::Message::Binary(bin)),
      web::ws::Frame::Ping(msg) => Some(web::ws::Message::Pong(msg)),
      web::ws::Frame::Close(reason) => Some(web::ws::Message::Close(reason)),
      _ => None,
    };
    futures::future::ready(Ok(response))
  });

  Ok(service)
}

async fn ws_index(req: web::HttpRequest) -> Result<web::HttpResponse, web::Error> {
  web::ws::start(req, fn_factory_with_config(ws_service)).await
}

#[ntex::main]
async fn main() -> std::io::Result<()> {
  default_provider()
    .install_default()
    .expect("Failed to install default CryptoProvider");

  let addr: SocketAddr = "0.0.0.0:80".parse().unwrap();
  let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
  socket.set_reuse_address(true)?;

  #[cfg(target_os = "linux")]
  socket.set_reuse_port(true)?;

  socket.bind(&addr.into())?;
  socket.listen(addr.port().into())?;

  let std_listener = std::net::TcpListener::from(socket);
  std_listener.set_nonblocking(true)?;

  let global_state = Arc::new(GlobalState {
    symbol_map: Mutex::new(HashMap::new()),
    worker_channels: Mutex::new(HashMap::new()),
    next_worker: AtomicU32::new(0),
    last_id: AtomicU32::new(0)
  });

  let worker_global_state = global_state.clone();

  Server::build()
    .workers(num_cpus::get())
    .on_worker_start(move || on_worker_start(worker_global_state.clone()))
    .listen("http", std_listener, move |_| {
      HttpService::build().finish(
        App::new()
          .state(global_state.clone())
          .wrap(middleware::Logger::default())
          .service(web::resource("/ws").route(web::get().to(ws_index)))
          .service((start_arbitrage, no_params, symbols, start_monitor))
          .service(
            web::resource("/resource2/index.html")
              .wrap(ntex::util::timeout::Timeout::new(ntex::time::Millis(5000)))
              .wrap(middleware::DefaultHeaders::new().header("X-Version-R2", "0.3"))
              .default_service(web::route().to(|| async { HttpResponse::MethodNotAllowed() }))
              .route(web::get().to(index_async)),
          )
          .service(web::resource("/test1.html").to(|| async { "Test\r\n" })),
      )
    })?
    .run()
    .await
}
