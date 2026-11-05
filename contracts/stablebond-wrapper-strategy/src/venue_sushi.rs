//! Sushi (Uniswap v3 style) pool called directly. Ordering is load-bearing: read the oracle
//! hints, THEN grant the pool's token pull, THEN swap. A grant is scoped to the next
//! invocation's subtree, so a grant made before the hints read is consumed by it
//! (`aggregator-v3 venues/sushi.rs:44-53`). The price bound is the extreme tick bound for
//! the direction, so the pool only stops short on exhausted liquidity and `min_out` is the
//! guarantee. The received leg is the negative field of `SwapResult`.

use crate::abi::{SushiPoolClient, SwapResult};
use crate::venue::grant_transfer;
use defindex_strategy_core::StrategyError;
use soroban_sdk::{Address, Env, U256};

/// Uniswap v3 `MIN_SQRT_RATIO + 1`, used when selling token0 (`zero_for_one`).
const MIN_SQRT_RATIO_PLUS_1: u64 = 4_295_128_740;
/// Uniswap v3 `MAX_SQRT_RATIO - 1` = 1461446703485210103287273052203988822378723970341 as
/// `U256::from_parts(hi_hi, hi_lo, lo_hi, lo_lo)` pieces.
const MAX_SQRT_RATIO_MINUS_1: (u64, u64, u64, u64) =
    (0, 0xfffd_8963, 0xefd1_fc6a_5064_8849, 0x5d95_1d52_6398_8d25);

fn bound(e: &Env, zero_for_one: bool) -> U256 {
    if zero_for_one {
        U256::from_parts(e, 0, 0, 0, MIN_SQRT_RATIO_PLUS_1)
    } else {
        let (a, b, c, d) = MAX_SQRT_RATIO_MINUS_1;
        U256::from_parts(e, a, b, c, d)
    }
}

/// The leg the pool paid out, as a positive amount. `None` when it is not a real fill.
fn received(r: &SwapResult, zero_for_one: bool) -> Option<i128> {
    let raw = if zero_for_one { r.amount1 } else { r.amount0 };
    let v = raw.checked_neg()?;
    if v <= 0 {
        None
    } else {
        Some(v)
    }
}

/// Non-mutating quote. `None` on any failure (paused pool, exhausted range, panic).
pub fn try_quote(e: &Env, pool: &Address, zero_for_one: bool, amount_in: i128) -> Option<i128> {
    if amount_in <= 0 {
        return None;
    }
    let r = SushiPoolClient::new(e, pool)
        .try_quote_exact_input(&zero_for_one, &amount_in, &bound(e, zero_for_one))
        .ok()?
        .ok()?;
    received(&r, zero_for_one)
}

/// Executed exact-in swap. Any venue failure or short fill is `InternalSwapError`.
pub fn swap(
    e: &Env,
    pool: &Address,
    token_in: &Address,
    zero_for_one: bool,
    amount_in: i128,
    min_out: i128,
) -> Result<i128, StrategyError> {
    let this = e.current_contract_address();
    let client = SushiPoolClient::new(e, pool);
    let hints = client
        .try_get_oracle_hints()
        .map_err(|_| StrategyError::InternalSwapError)?
        .map_err(|_| StrategyError::InternalSwapError)?;
    grant_transfer(e, &this, token_in, pool, amount_in);
    let r = client
        .try_swap(&this, &this, &zero_for_one, &amount_in, &bound(e, zero_for_one), &hints)
        .map_err(|_| StrategyError::InternalSwapError)?
        .map_err(|_| StrategyError::InternalSwapError)?;
    let got = received(&r, zero_for_one).ok_or(StrategyError::InternalSwapError)?;
    if got < min_out {
        return Err(StrategyError::InternalSwapError);
    }
    Ok(got)
}
