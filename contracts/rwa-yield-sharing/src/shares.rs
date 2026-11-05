//! Virtual-offset share accounting (OpenZeppelin ERC-4626 v5 style).
//!
//! `VIRTUAL_SHARES` is added to the shares side of every conversion, making the classic
//! first-depositor inflation attack uneconomical without burning any real shares. NAV is
//! the **USDC-denominated** value of the strategy's held RWA (see `pricing.rs`), so there
//! is no rate cache.
//!
//! Rounding is always conservative toward the strategy: floor on mint, ceil on burn,
//! floor on user-facing balance. See `docs/strategies/rwa-yield-sharing/architecture.md`.

use crate::constants::{SCALAR_12, VIRTUAL_SHARES};
use defindex_strategy_core::StrategyError;
use soroban_fixed_point_math::FixedPoint;

#[inline]
fn shares_denominator(total_shares: i128) -> Result<i128, StrategyError> {
    total_shares
        .checked_add(VIRTUAL_SHARES)
        .ok_or(StrategyError::UnderflowOverflow)
}

#[inline]
fn nav_denominator(nav: i128) -> Result<i128, StrategyError> {
    // The `+1` virtual asset pairs with the virtual shares so the ratio is well-defined
    // when NAV is zero, and the attacker can never drive a victim's mint to zero shares.
    nav.checked_add(1).ok_or(StrategyError::UnderflowOverflow)
}

/// assets -> shares, rounding DOWN (deposit mint). `nav_before` is the strategy NAV prior
/// to the deposit; `assets_in` is the USDC deposited (mints at par into idle).
pub fn assets_to_shares_down(
    assets_in: i128,
    total_shares: i128,
    nav_before: i128,
) -> Result<i128, StrategyError> {
    assets_in
        .fixed_mul_floor(shares_denominator(total_shares)?, nav_denominator(nav_before)?)
        .ok_or(StrategyError::ArithmeticError)
}

/// assets -> shares, rounding UP (withdraw burn) so we never under-burn the withdrawer.
pub fn assets_to_shares_up(
    amount: i128,
    total_shares: i128,
    nav: i128,
) -> Result<i128, StrategyError> {
    amount
        .fixed_mul_ceil(shares_denominator(total_shares)?, nav_denominator(nav)?)
        .ok_or(StrategyError::ArithmeticError)
}

/// shares -> assets, rounding DOWN (user-facing balance query).
pub fn shares_to_assets_down(
    shares: i128,
    total_shares: i128,
    nav: i128,
) -> Result<i128, StrategyError> {
    shares
        .fixed_mul_floor(nav_denominator(nav)?, shares_denominator(total_shares)?)
        .ok_or(StrategyError::ArithmeticError)
}

/// Price per share scaled by `SCALAR_12`, for the harvest event's APY metric. Uses the
/// same virtual-offset denominator as the share math so it stays internally consistent.
pub fn price_per_share(nav: i128, total_shares: i128) -> Result<i128, StrategyError> {
    nav.fixed_mul_floor(SCALAR_12, shares_denominator(total_shares)?)
        .ok_or(StrategyError::ArithmeticError)
}
