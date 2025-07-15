use crate::{
  exchange::mexc::MexcExchange,
  worker::{
    commands::{Request, StartArbitrage},
    state::GlobalState,
    worker_loop,
  },
};
use async_channel::unbounded;
use futures::TryStreamExt;
use ntex::{
  Service, fn_service,
  http::HttpService,
  server::Server,
  service::fn_factory_with_config,
  util::BytesMut,
  web::{self, App, HttpRequest, HttpResponse, middleware},
};
use rustls::crypto::aws_lc_rs::default_provider;
use socket2::{Domain, Protocol, Socket, Type};
use std::{
  collections::HashMap,
  net::SocketAddr,
  sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
  },
  vec,
};

pub mod arbitrage;
pub mod base;
pub mod exchange;
pub mod test;
pub mod worker;

#[web::post("/arbitrage/{symbol}/start")]
async fn index(
  _: HttpRequest,
  symbol: web::types::Path<String>,
  mut payload: web::types::Payload,
  data: web::types::State<GlobalState>,
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
    let _ = tx.send(Request::Start(command)).await;

    HttpResponse::Ok().body(format!("Started!"))
  } else {
    HttpResponse::InternalServerError().body("Worker not found")
  }
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
    last_id: AtomicU32::new(0),
    exchanges: vec![],
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
          .service((index, no_params, symbols))
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
