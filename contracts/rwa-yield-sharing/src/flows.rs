//! Trade sizing shared by deposit's tranche, keeper `invest`/`divest` and `withdraw`.

use crate::constants::STRATEGY_NAME;
use crate::events::InvestEvent;
use crate::math;
use crate::pricing::{self, PricingState};
use crate::storage::{self, Config, Params};
use crate::venue;
use defindex_strategy_core::StrategyError;
use soroban_sdk::{Env, String};

/// Minimum RWA to accept for `usdc_in`: the quantity the guarded oracle rate implies minus
/// `slippage_bps`.
pub fn min_rwa_out(e: &Env, config: &Config, params: &Params, rate: i128, usdc_in: i128) -> Result<i128, StrategyError> {
    let expected = math::qty_for(e, usdc_in, rate, config.rwa_decimals)?;
    math::apply_bps(expected, -(params.slippage_bps as i128))
}

/// Maximum RWA to sell to source `usdc_out`: implied quantity plus `slippage_bps`, **both
/// steps rounded up**. The sale is exact-in with `min_out = usdc_out`, so a floored implied
/// quantity is worth one stroop less than `usdc_out` even on a fair, fee-free pool and the
/// sale reverts; a floored slippage step leaves the cap one raw unit
/// tight the same way. Never above the held RWA when `usdc_out <= value_of(held)`, because
/// `value_of` floors; the callers clamp to held anyway.
pub fn max_rwa_in(e: &Env, config: &Config, params: &Params, rate: i128, usdc_out: i128) -> Result<i128, StrategyError> {
    let expected = math::qty_for_ceil(e, usdc_out, rate, config.rwa_decimals)?;
    math::apply_bps_ceil(expected, params.slippage_bps as i128)
}

/// USDC that may still be invested under `max_invested` at `rate`.
pub fn invested_room(e: &Env, config: &Config, params: &Params, rate: i128, held: i128) -> Result<i128, StrategyError> {
    let invested = math::value_of(e, held, rate, config.rwa_decimals)?;
    Ok(params.max_invested.saturating_sub(invested).max(0))
}

/// Buys RWA with `usdc_in` at `min_rwa_out`, updates `ExpectedRwa`, emits `InvestEvent`.
/// Returns the RWA received.
pub fn buy_rwa(e: &Env, config: &Config, params: &Params, rate: i128, held: i128, usdc_in: i128) -> Result<i128, StrategyError> {
    let min_out = min_rwa_out(e, config, params, rate, usdc_in)?;
    let received = venue::buy(e, config, &config.venue, usdc_in, min_out)?;
    storage::set_expected_rwa(e, held.checked_add(received).ok_or(StrategyError::UnderflowOverflow)?);
    InvestEvent { strategy_name: String::from_str(e, STRATEGY_NAME), usdc_in, rwa_out: received }.publish(e);
    Ok(received)
}

/// one best-effort tranche after a deposit. Returns `Ok(())` without buying
/// unless every condition holds and the pre-flight `try_` buy quote clears the floor. A
/// venue that then fails inside `swap` propagates the error and reverts the deposit: no USDC
/// has left the vault, so that is fail-closed and acceptable.
///
/// Sized from `deposited`, the USDC this deposit just added, never from aggregate idle: the
/// idle balance is the withdrawal buffer earlier deposits and OTC refills left, and one small
/// deposit must not spend it. `deposited` is already idle, so it never
/// exceeds what the strategy holds.
pub fn auto_invest_tranche(e: &Env, config: &Config, params: &Params, state: &PricingState, held: i128, deposited: i128) -> Result<(), StrategyError> {
    if !params.auto_invest || params.paused || !pricing::can_buy(state) {
        return Ok(());
    }
    let room = invested_room(e, config, params, state.rate, held)?;
    let tranche = deposited.min(params.max_swap).min(room);
    if tranche <= 0 {
        return Ok(());
    }
    let min_out = min_rwa_out(e, config, params, state.rate, tranche)?;
    match venue::try_quote_buy(e, &config.venue, tranche) {
        Some(quote) if quote >= min_out => {}
        _ => return Ok(()),
    }
    buy_rwa(e, config, params, state.rate, held, tranche)?;
    Ok(())
}
