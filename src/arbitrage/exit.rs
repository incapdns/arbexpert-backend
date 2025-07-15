use rust_decimal::{prelude::FromPrimitive, prelude::ToPrimitive, Decimal};

use crate::arbitrage::common::{
  clean_residual, find_max_price, ArbitrageOrder, ExitRequest, ExitResult, MaxPrice
};

pub fn do_exit_arbitrage(request: ExitRequest) -> ExitResult {
  let mut spot_clone = request
    .spot_orders
    .iter()
    .map(|&(p, q)| (Decimal::from_f64(p).unwrap(), Decimal::from_f64(q).unwrap()))
    .collect::<Vec<_>>();
  let mut future_clone = request
    .future_orders
    .iter()
    .map(|&(p, q)| (Decimal::from_f64(p).unwrap(), Decimal::from_f64(q).unwrap()))
    .collect::<Vec<_>>();

  let mut i = 0;
  let mut j = 0;
  let mut available = Decimal::from_f64(request.executed).unwrap();

  let mut spot_results: Vec<ArbitrageOrder> = Vec::new();
  let mut future_results: Vec<ArbitrageOrder> = Vec::new();
  let eps = Decimal::new(1, 12);

  while i < spot_clone.len()
    && j < future_clone.len()
    && clean_residual(available, eps) > Decimal::ZERO
  {
    let spot_price = spot_clone[i].0;
    let spot_vol = spot_clone[i].1;
    let future_price = future_clone[j].0;
    let future_vol = future_clone[j].1 * Decimal::from_f64(request.contract_size).unwrap();

    let diff = (spot_price - future_price) / future_price * Decimal::new(100, 0);

    if diff < Decimal::from_f64(request.percent).unwrap() {
      break;
    }

    let qty = available.min(spot_vol).min(future_vol);
    available -= qty;

    // Spot result
    if let Some(last) = spot_results.last_mut() {
      if (last.price - spot_price.to_f64().unwrap()).abs() < f64::EPSILON {
        last.quantity += qty.to_f64().unwrap();
      } else {
        spot_results.push(ArbitrageOrder {
          price: spot_price.to_f64().unwrap(),
          quantity: qty.to_f64().unwrap(),
        });
      }
    } else {
      spot_results.push(ArbitrageOrder {
        price: spot_price.to_f64().unwrap(),
        quantity: qty.to_f64().unwrap(),
      });
    }

    // Future result
    if let Some(last) = future_results.last_mut() {
      if (last.price - future_price.to_f64().unwrap()).abs() < f64::EPSILON {
        last.quantity += qty.to_f64().unwrap();
      } else {
        future_results.push(ArbitrageOrder {
          price: future_price.to_f64().unwrap(),
          quantity: qty.to_f64().unwrap(),
        });
      }
    } else {
      future_results.push(ArbitrageOrder {
        price: future_price.to_f64().unwrap(),
        quantity: qty.to_f64().unwrap(),
      });
    }

    if qty > Decimal::ZERO {
      spot_clone[i].1 -= qty;
      future_clone[j].1 -= qty;
    }

    if clean_residual(spot_clone[i].1, eps) == Decimal::ZERO {
      i += 1;
    }
    if clean_residual(future_clone[j].1, eps) == Decimal::ZERO {
      j += 1;
    }
  }

  let completed = clean_residual(available, eps) == Decimal::ZERO;

  let max_price = find_max_price(
    &request.future_orders,
    &request.spot_orders,
    request.percent,
  );

  ExitResult {
    spot_orders: spot_results,
    future_orders: future_results,
    max_price: MaxPrice {
      // note a inversão de campos para espelhar o TS
      spot: max_price.as_ref().map_or(0.0, |m| m.future),
      future: max_price.as_ref().map_or(0.0, |m| m.spot),
    },
    executed: request.executed - available.to_f64().unwrap(),
    completed,
  }
}
