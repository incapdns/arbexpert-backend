use crate::exchange::mexc::private::MexcExchangePrivate;
use crate::exchange::mexc::public::MexcExchangePublic;
use crate::exchange::mexc::utils::MexcExchangeUtils;
use hmac::Hmac;
use sha2::Sha256;
use std::rc::Rc;

pub mod private;
pub mod public;
pub mod utils;

#[allow(dead_code)]
type HmacSha256 = Hmac<Sha256>;

pub mod mexc_proto {
  include!(concat!(env!("PROTO_OUT"), "/mexc.rs"));
}

pub struct MexcExchange {
  private: MexcExchangePrivate,
  public: MexcExchangePublic,
  utils: Rc<MexcExchangeUtils>
}