//! Aqua liquidity pool called directly. The pool pulls `in_amount` from `user` with
//! `token_in.transfer(user, pool, amount)` at depth 2, so the strategy grants
//! exactly that transfer before the call. `user.require_auth()` inside the pool is
//! auto-authorized because the strategy is the direct invoker.

use crate::abi::AquaPoolClient;
use crate::venue::grant_transfer;
use defindex_strategy_core::StrategyError;
use soroban_sdk::{Address, Env};

/// Non-mutating quote for `amount_in` of token `in_idx` into token `out_idx`. `None` on any
/// failure or a non-positive result.
pub fn try_quote(e: &Env, pool: &Address, in_idx: u32, out_idx: u32, amount_in: i128) -> Option<i128> {
    let amount = u128::try_from(amount_in).ok()?;
    if amount == 0 {
        return None;
    }
    let out = AquaPoolClient::new(e, pool).try_estimate_swap(&in_idx, &out_idx, &amount).ok()?.ok()?;
    let out = i128::try_from(out).ok()?;
    if out <= 0 {
        None
    } else {
        Some(out)
    }
}

/// Executed exact-in swap of `amount_in` `token_in` for at least `min_out`, proceeds to the
/// strategy. Any venue failure or short fill is `InternalSwapError`.
pub fn swap(
    e: &Env,
    pool: &Address,
    token_in: &Address,
    in_idx: u32,
    out_idx: u32,
    amount_in: i128,
    min_out: i128,
) -> Result<i128, StrategyError> {
    let this = e.current_contract_address();
    let amount = u128::try_from(amount_in).map_err(|_| StrategyError::ArithmeticError)?;
    let out_min = u128::try_from(min_out).map_err(|_| StrategyError::ArithmeticError)?;
    grant_transfer(e, &this, token_in, pool, amount_in);
    let out = AquaPoolClient::new(e, pool)
        .try_swap(&this, &in_idx, &out_idx, &amount, &out_min)
        .map_err(|_| StrategyError::InternalSwapError)?
        .map_err(|_| StrategyError::InternalSwapError)?;
    let received = i128::try_from(out).map_err(|_| StrategyError::ArithmeticError)?;
    // The pool enforces out_min itself; re-assert so a venue that does not is still safe.
    if received < min_out {
        return Err(StrategyError::InternalSwapError);
    }
    Ok(received)
}
