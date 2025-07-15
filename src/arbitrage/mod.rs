pub mod common;
pub mod entry;
pub mod exit;

use common::{ArbitrageRequest, ArbitrageResult};
use entry::do_entry_arbitrage;
use exit::do_exit_arbitrage;

pub fn do_arbitrage(request: ArbitrageRequest) -> ArbitrageResult {
  match request {
    ArbitrageRequest::Entry(r) => ArbitrageResult::Entry(do_entry_arbitrage(r)),
    ArbitrageRequest::Exit(r) => ArbitrageResult::Exit(do_exit_arbitrage(r)),
  }
}