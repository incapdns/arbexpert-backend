use rust_decimal::Decimal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArbitrageDirection {
  Entry,
  Exit,
}

#[derive(Debug, Clone)]
pub struct ArbitrageOrder {
  pub price: f64,
  pub quantity: f64,
}

#[derive(Debug, Clone)]
pub struct MaxPrice {
  pub spot: f64,
  pub future: f64,
}

#[derive(Debug, Clone)]
pub struct CommonResult {
  pub spot_orders: Vec<ArbitrageOrder>,
  pub future_orders: Vec<ArbitrageOrder>,
  pub max_price: MaxPrice,
  pub executed: f64,
  pub completed: bool,
}

#[derive(Debug, Clone)]
pub struct EntryResult {
  pub spot_orders: Vec<ArbitrageOrder>,
  pub future_orders: Vec<ArbitrageOrder>,
  pub max_price: MaxPrice,
  pub executed: f64,
  pub completed: bool,
  pub profit_percent: f64,
}

#[derive(Debug, Clone)]
pub struct ExitResult {
  pub spot_orders: Vec<ArbitrageOrder>,
  pub future_orders: Vec<ArbitrageOrder>,
  pub max_price: MaxPrice,
  pub executed: f64,
  pub completed: bool,
}

#[derive(Debug, Clone)]
pub struct CommonRequest {
  pub spot_orders: Vec<(f64, f64)>,
  pub future_orders: Vec<(f64, f64)>,
  pub percent: f64,
  pub contract_size: f64,
}

#[derive(Debug, Clone)]
pub struct EntryRequest {
  pub spot_orders: Vec<(f64, f64)>,
  pub future_orders: Vec<(f64, f64)>,
  pub amount: f64,
  pub margin_quantity_percent: f64,
  pub percent: f64,
  pub contract_size: f64,
}

#[derive(Debug, Clone)]
pub struct ExitRequest {
  pub spot_orders: Vec<(f64, f64)>,
  pub future_orders: Vec<(f64, f64)>,
  pub executed: f64,
  pub percent: f64,
  pub contract_size: f64,
}

pub enum ArbitrageRequest {
  Entry(EntryRequest),
  Exit(ExitRequest),
}

pub enum ArbitrageResult {
  Entry(EntryResult),
  Exit(ExitResult),
}

/// Se o valor absoluto for menor que `epsilon`, zera-o.
pub fn clean_residual(value: Decimal, epsilon: Decimal) -> Decimal {
  if value.abs() < epsilon {
    Decimal::ZERO
  } else {
    value
  }
}

/// Busca o par (increasing, decreasing) que atinge exatamente (ou minimamente acima) `percentage`.
pub fn find_max_price(
  increasing: &[(f64, f64)],
  decreasing: &[(f64, f64)],
  percentage: f64,
) -> Option<MaxPrice> {
  let mut best: Option<MaxPrice> = None;
  let mut min_excess = f64::INFINITY;

  let mut j = decreasing.len() as isize - 1;
  for &(inc_price, _) in increasing.iter() {
    if j < 0 {
      break;
    }
    let required = inc_price * (1.0 + percentage / 100.0);
    while j >= 0 && decreasing[j as usize].0 < required {
      j -= 1;
    }
    if j < 0 {
      break;
    }
    let dec_price = decreasing[j as usize].0;
    let diff_percentage = (dec_price - inc_price) / inc_price * 100.0;
    let excess = diff_percentage - percentage;
    if excess < min_excess {
      min_excess = excess;
      best = Some(MaxPrice {
        spot: inc_price,
        future: dec_price,
      });
    }
  }
  best
}

/// Verifica se `target` está fora da tolerância `[base – base*percent/100, base + base*percent/100]`.
pub fn is_outside_tolerance(base: Decimal, target: Decimal, percent: Decimal) -> bool {
  let tolerance = base * percent / Decimal::new(100, 0);
  let lower = base - tolerance;
  let upper = base + tolerance;
  target < lower || target > upper
}