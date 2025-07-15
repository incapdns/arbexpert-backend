use rust_decimal::{prelude::FromPrimitive, prelude::ToPrimitive, Decimal};

use crate::arbitrage::common::{
  ArbitrageOrder, EntryRequest, EntryResult, MaxPrice, clean_residual, find_max_price,
  is_outside_tolerance,
};

pub fn do_entry_arbitrage(request: EntryRequest) -> EntryResult {
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

  let mut total_spot = Decimal::ZERO;
  let mut total_future = Decimal::ZERO;
  let mut available = Decimal::from_f64(request.amount).unwrap();

  let mut spot_results: Vec<ArbitrageOrder> = Vec::new();
  let mut future_results: Vec<ArbitrageOrder> = Vec::new();

  let mut qty = Decimal::ZERO;
  let mut completed = false;
  let eps = Decimal::new(1, 12);

  while i < spot_clone.len()
    && j < future_clone.len()
    && clean_residual(available, eps) > Decimal::ZERO
  {
    let spot_price = spot_clone[i].0;
    let spot_vol = spot_clone[i].1;
    let future_price = future_clone[j].0;
    let future_vol = future_clone[j].1 * Decimal::from_f64(request.contract_size).unwrap();

    let diff = (future_price - spot_price) / spot_price * Decimal::new(100, 0);

    if clean_residual(diff, eps) == Decimal::ZERO
      || diff < Decimal::from_f64(request.percent).unwrap()
    {
      break;
    }

    let max_qty = available / spot_price;
    let current_qty = max_qty.min(spot_vol).min(future_vol);

    if clean_residual(current_qty, eps) == Decimal::ZERO {
      break;
    }

    // Executa spot
    total_spot += spot_price * current_qty;
    available -= spot_price * current_qty;

    if let Some(last) = spot_results.last_mut() {
      if (last.price - spot_price.to_f64().unwrap()).abs() < f64::EPSILON {
        last.quantity += current_qty.to_f64().unwrap();
      } else {
        spot_results.push(ArbitrageOrder {
          price: spot_price.to_f64().unwrap(),
          quantity: current_qty.to_f64().unwrap(),
        });
      }
    } else {
      spot_results.push(ArbitrageOrder {
        price: spot_price.to_f64().unwrap(),
        quantity: current_qty.to_f64().unwrap(),
      });
    }

    // Executa future
    total_future += future_price * current_qty;
    if let Some(last) = future_results.last_mut() {
      if (last.price - future_price.to_f64().unwrap()).abs() < f64::EPSILON {
        last.quantity += current_qty.to_f64().unwrap();
      } else {
        future_results.push(ArbitrageOrder {
          price: future_price.to_f64().unwrap(),
          quantity: current_qty.to_f64().unwrap(),
        });
      }
    } else {
      future_results.push(ArbitrageOrder {
        price: future_price.to_f64().unwrap(),
        quantity: current_qty.to_f64().unwrap(),
      });
    }

    spot_clone[i].1 -= current_qty;
    future_clone[j].1 -= current_qty;

    if clean_residual(spot_clone[i].1, eps) == Decimal::ZERO {
      i += 1;
    }
    if clean_residual(future_clone[j].1, eps) == Decimal::ZERO {
      j += 1;
    }

    qty += current_qty;

    completed = !is_outside_tolerance(
      Decimal::from_f64(request.amount).unwrap(),
      total_spot,
      Decimal::from_f64(request.margin_quantity_percent).unwrap(),
    ) && !is_outside_tolerance(
      Decimal::from_f64(request.amount).unwrap(),
      total_future,
      Decimal::from_f64(request.margin_quantity_percent).unwrap(),
    );
  }

  // Cálculo de lucro
  let profit = total_future - total_spot;
  let profit_percent = if total_spot > Decimal::ZERO {
    profit / total_spot * Decimal::new(100, 0)
  } else {
    Decimal::ZERO
  };

  if clean_residual(qty, eps) == Decimal::ZERO {
    return EntryResult {
      spot_orders: vec![],
      future_orders: vec![],
      max_price: MaxPrice {
        spot: 0.0,
        future: 0.0,
      },
      executed: 0.0,
      completed: false,
      profit_percent: 0.0,
    };
  }

  let max_price = find_max_price(
    &request.spot_orders,
    &request.future_orders,
    request.percent,
  );

  EntryResult {
    spot_orders: spot_results,
    future_orders: future_results,
    max_price: MaxPrice {
      spot: max_price.as_ref().map_or(0.0, |m| m.spot),
      future: max_price.as_ref().map_or(0.0, |m| m.future),
    },
    executed: qty.to_f64().unwrap(),
    completed,
    profit_percent: profit_percent.to_f64().unwrap(),
  }
}
