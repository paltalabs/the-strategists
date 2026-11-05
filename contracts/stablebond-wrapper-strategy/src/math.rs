//! The price scale and the small bps helpers. Every function is pure; every
//! intermediate that can exceed i128 (18-decimal RWAs at 8M positions) goes through I256.

use crate::constants::{BPS_DENOMINATOR, MAX_DECIMALS, SCALAR_12};
use defindex_strategy_core::StrategyError;
use soroban_sdk::{Env, I256};

/// `10^n` as an `I256`. `n` is a cached decimals value bounded at construction; the guard is
/// defense-in-depth so no caller can reintroduce an unbounded loop.
pub fn pow10(e: &Env, n: u32) -> Result<I256, StrategyError> {
    if n > 2 * MAX_DECIMALS {
        return Err(StrategyError::ArithmeticError);
    }
    let mut acc = I256::from_i128(e, 1);
    let ten = I256::from_i128(e, 10);
    let mut i = 0;
    while i < n {
        acc = acc.mul(&ten);
        i += 1;
    }
    Ok(acc)
}

/// `10^decimals * SCALAR_12`: the denominator of the rate scale for a token.
fn scale(e: &Env, decimals: u32) -> Result<I256, StrategyError> {
    Ok(pow10(e, decimals)?.mul(&I256::from_i128(e, SCALAR_12)))
}

fn to_i128(v: I256) -> Result<i128, StrategyError> {
    v.to_i128().ok_or(StrategyError::ArithmeticError)
}

/// `rate` from an oracle price quoted in USD per whole RWA with `price_decimals`:
/// `price * 10^asset_decimals * SCALAR_12 / 10^price_decimals`.
pub fn rate_from_price(
    e: &Env,
    price: i128,
    price_decimals: u32,
    asset_decimals: u32,
) -> Result<i128, StrategyError> {
    if price <= 0 {
        return Err(StrategyError::InvalidArgument);
    }
    let num = I256::from_i128(e, price).mul(&scale(e, asset_decimals)?);
    to_i128(num.div(&pow10(e, price_decimals)?))
}

/// USDC stroops that `qty` raw RWA units are worth at `rate`, floored. Zero for a zero or
/// negative rate (no anchor and no live oracle values the RWA at nothing).
pub fn value_of(e: &Env, qty: i128, rate: i128, rwa_decimals: u32) -> Result<i128, StrategyError> {
    if qty <= 0 || rate <= 0 {
        return Ok(0);
    }
    let num = I256::from_i128(e, qty).mul(&I256::from_i128(e, rate));
    to_i128(num.div(&scale(e, rwa_decimals)?))
}

/// Raw RWA units worth `usdc` stroops at `rate`, floored. Callers refuse `rate == 0` before
/// sizing a trade; this returns `DivisionByZero` as the backstop.
pub fn qty_for(e: &Env, usdc: i128, rate: i128, rwa_decimals: u32) -> Result<i128, StrategyError> {
    if rate <= 0 {
        return Err(StrategyError::DivisionByZero);
    }
    if usdc <= 0 {
        return Ok(0);
    }
    let num = I256::from_i128(e, usdc).mul(&scale(e, rwa_decimals)?);
    to_i128(num.div(&I256::from_i128(e, rate)))
}

/// Raw RWA units worth `usdc` stroops at `rate`, rounded up: the fewest units whose
/// `value_of` is at least `usdc`. Only for a quantity the strategy sells to *raise* `usdc`
/// (`flows::max_rwa_in`): the floor there undershoots, so a fair pool pays one stroop short
/// of `min_out`. At most one raw unit above `qty_for`.
pub fn qty_for_ceil(e: &Env, usdc: i128, rate: i128, rwa_decimals: u32) -> Result<i128, StrategyError> {
    if rate <= 0 {
        return Err(StrategyError::DivisionByZero);
    }
    if usdc <= 0 {
        return Ok(0);
    }
    let num = I256::from_i128(e, usdc).mul(&scale(e, rwa_decimals)?);
    let den = I256::from_i128(e, rate);
    let one = I256::from_i128(e, 1);
    // Both operands are positive here, so `(num + den - 1) / den` is the ceiling.
    to_i128(num.add(&den).sub(&one).div(&den))
}

/// `|a - b| * 10_000 / max(a, b)`: the disagreement between two rates in bps.
///
/// **Precondition: both inputs are in `[0, MAX_RATE]`.** That is what keeps the unchecked
/// `(a - b).abs()` safe: the difference of two values in that range cannot overflow i128 and
/// can never be `i128::MIN`, whose `.abs()` would panic. Every caller satisfies it because a
/// rate only reaches pricing after the `MAX_RATE` filter in `oracles::read_rate`, and the
/// anchor is a past rate; the `* 10_000` that follows is checked anyway.
pub fn spread_bps(a: i128, b: i128) -> Result<i128, StrategyError> {
    let hi = a.max(b);
    if hi <= 0 {
        return Ok(0);
    }
    (a - b)
        .abs()
        .checked_mul(BPS_DENOMINATOR)
        .ok_or(StrategyError::ArithmeticError)?
        .checked_div(hi)
        .ok_or(StrategyError::ArithmeticError)
}

fn bps_factor(delta_bps: i128) -> Result<i128, StrategyError> {
    let factor = BPS_DENOMINATOR
        .checked_add(delta_bps)
        .ok_or(StrategyError::ArithmeticError)?;
    if factor < 0 {
        return Err(StrategyError::InvalidArgument);
    }
    Ok(factor)
}

/// `x * (10_000 + delta_bps) / 10_000`, floored. Negative `delta_bps` is a haircut.
pub fn apply_bps(x: i128, delta_bps: i128) -> Result<i128, StrategyError> {
    x.checked_mul(bps_factor(delta_bps)?)
        .ok_or(StrategyError::ArithmeticError)?
        .checked_div(BPS_DENOMINATOR)
        .ok_or(StrategyError::ArithmeticError)
}

/// `x * (10_000 + delta_bps) / 10_000`, rounded up. For quantities the strategy gives away.
pub fn apply_bps_ceil(x: i128, delta_bps: i128) -> Result<i128, StrategyError> {
    let num = x
        .checked_mul(bps_factor(delta_bps)?)
        .ok_or(StrategyError::ArithmeticError)?;
    num.checked_add(BPS_DENOMINATOR - 1)
        .ok_or(StrategyError::ArithmeticError)?
        .checked_div(BPS_DENOMINATOR)
        .ok_or(StrategyError::ArithmeticError)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    const ONE: i128 = 10_000_000; // USDC, 7 dp
    const ONE_18: i128 = 1_000_000_000_000_000_000;

    #[test]
    fn rate_from_price_reflector_14dp() {
        // 1.0745 USD per RWA at 14 dp, USDC 7 dp -> 1.0745e7 * 1e12
        let e = Env::default();
        let r = rate_from_price(&e, 107_450_000_000_000, 14, 7).unwrap();
        assert_eq!(r, 10_745_000 * SCALAR_12);
    }

    #[test]
    fn rate_from_price_redstone_8dp() {
        let e = Env::default();
        let r = rate_from_price(&e, 107_450_000, 8, 7).unwrap();
        assert_eq!(r, 10_745_000 * SCALAR_12);
    }

    #[test]
    fn rate_from_price_rejects_non_positive() {
        let e = Env::default();
        assert_eq!(rate_from_price(&e, 0, 8, 7), Err(StrategyError::InvalidArgument));
        assert_eq!(rate_from_price(&e, -1, 8, 7), Err(StrategyError::InvalidArgument));
    }

    #[test]
    fn value_of_and_qty_for_round_trip_7dp() {
        let e = Env::default();
        let rate = 10_745_000 * SCALAR_12;
        assert_eq!(value_of(&e, ONE, rate, 7).unwrap(), 10_745_000);
        assert_eq!(qty_for(&e, 10_745_000, rate, 7).unwrap(), ONE);
        // floor: 1 USDC buys 0.9306654 RWA
        assert_eq!(qty_for(&e, ONE, rate, 7).unwrap(), 9_306_654);
    }

    #[test]
    fn value_of_and_qty_for_18dp_position_of_8m() {
        // 8M deJTRSY (18 dp) at 1.0367: 8.2936M USDC, no overflow.
        let e = Env::default();
        let rate = 10_367_000 * SCALAR_12;
        let qty = 8_000_000 * ONE_18;
        assert_eq!(value_of(&e, qty, rate, 18).unwrap(), 8_293_600 * ONE);
        assert_eq!(qty_for(&e, 8_293_600 * ONE, rate, 18).unwrap(), qty);
    }

    #[test]
    fn value_of_zero_rate_or_qty_is_zero_and_qty_for_zero_rate_errors() {
        let e = Env::default();
        assert_eq!(value_of(&e, ONE, 0, 7).unwrap(), 0);
        assert_eq!(value_of(&e, 0, 5 * SCALAR_12, 7).unwrap(), 0);
        assert_eq!(qty_for(&e, ONE, 0, 7), Err(StrategyError::DivisionByZero));
        assert_eq!(qty_for(&e, 0, 5 * SCALAR_12, 7).unwrap(), 0);
    }

    #[test]
    fn qty_for_ceil_is_the_fewest_units_worth_the_target() {
        // 1.0745: 1 USDC is 9_306_654.2 raw units at 7 dp.
        let e = Env::default();
        let rate = 10_745_000 * SCALAR_12;
        assert_eq!(qty_for(&e, ONE, rate, 7).unwrap(), 9_306_654);
        assert_eq!(qty_for_ceil(&e, ONE, rate, 7).unwrap(), 9_306_655);
        assert_eq!(value_of(&e, 9_306_654, rate, 7).unwrap(), ONE - 1, "the floor is a stroop short");
        assert_eq!(value_of(&e, 9_306_655, rate, 7).unwrap(), ONE, "the ceiling covers it");
        // Exact quotients are not bumped.
        let r1 = 10_000_000 * SCALAR_12;
        assert_eq!(qty_for_ceil(&e, ONE, r1, 7).unwrap(), ONE);
        assert_eq!(qty_for_ceil(&e, 8_293_600 * ONE, 10_367_000 * SCALAR_12, 18).unwrap(), 8_000_000 * ONE_18);
        // Boundaries: 0 and 1 stroop, and the same backstop as the floor.
        assert_eq!(qty_for_ceil(&e, 0, rate, 7).unwrap(), 0);
        assert_eq!(qty_for_ceil(&e, 1, rate, 7).unwrap(), 1);
        assert_eq!(qty_for(&e, 1, rate, 7).unwrap(), 0);
        assert_eq!(qty_for_ceil(&e, ONE, 0, 7), Err(StrategyError::DivisionByZero));
        // 18 dp: still the fewest units covering the target, one above the floor.
        let c = qty_for_ceil(&e, ONE, rate, 18).unwrap();
        assert_eq!(c, qty_for(&e, ONE, rate, 18).unwrap() + 1);
        assert!(value_of(&e, c, rate, 18).unwrap() >= ONE);
        assert!(value_of(&e, c - 1, rate, 18).unwrap() < ONE);
    }

    #[test]
    fn spread_bps_is_symmetric_and_relative_to_the_larger() {
        assert_eq!(spread_bps(100, 99).unwrap(), 100);
        assert_eq!(spread_bps(99, 100).unwrap(), 100);
        assert_eq!(spread_bps(0, 0).unwrap(), 0);
        assert_eq!(spread_bps(100, 60).unwrap(), 4_000);
    }

    #[test]
    fn apply_bps_floor_and_ceil() {
        assert_eq!(apply_bps(10_000, -100).unwrap(), 9_900);
        assert_eq!(apply_bps(10_000, 100).unwrap(), 10_100);
        assert_eq!(apply_bps(3, -100).unwrap(), 2); // 2.97 floors
        assert_eq!(apply_bps_ceil(3, -100).unwrap(), 3); // 2.97 ceils
        assert_eq!(apply_bps_ceil(10_000, 50).unwrap(), 10_050);
        assert_eq!(apply_bps(1, -10_001), Err(StrategyError::InvalidArgument));
    }

    #[test]
    fn pow10_guards_the_exponent() {
        let e = Env::default();
        assert_eq!(pow10(&e, 36).unwrap().to_i128(), Some(10i128.pow(36)));
        assert_eq!(pow10(&e, 37), Err(StrategyError::ArithmeticError));
    }
}
