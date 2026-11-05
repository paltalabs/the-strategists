//! Allowance-based fills with a timelock-allowlisted counterparty at a price the keeper
//! passes per fill: the price agreed with the counterparty off-chain (an RFQ quote). The
//! contract only accepts it inside `rate * (1 ± otc_max_deviation_bps)` around the gated
//! rate, the oracle NAV (no pool price enters it), a band capped at `MAX_OTC_DEVIATION_BPS`, shipped `0` (so the shipped contract fills
//! at exactly the rate) and widened only through the 72 h `propose_params` wait. The keeper
//! chooses the price, never the band; the admin chooses the band, never a price. The strategy
//! pulls with `transfer_from` and pays with `transfer` in the same invocation, so either both
//! legs land or neither.
//! Pricing, NAV and `withdraw` never touch this path; it is never bounded by `max_swap`
//! because there is no pool impact. `invest` needs `pricing::can_buy`; `divest` needs only a
//! rate, so it refills idle at the anchor when no oracle is live or the price is frozen.

use crate::constants::STRATEGY_NAME;
use crate::events::{OtcDivestEvent, OtcInvestEvent};
use crate::flows;
use crate::math;
use crate::pricing;
use crate::storage::{self, Config, Params};
use defindex_strategy_core::StrategyError;
use soroban_sdk::{token::TokenClient, Address, Env, String};

fn name(e: &Env) -> String {
    String::from_str(e, STRATEGY_NAME)
}

/// `[lo, hi]` a fill price must sit in: `rate * (1 ± bps)`, both edges rounded inward so a
/// band of `0` admits exactly `rate` and no rounding ever widens it past the stored bps.
pub fn price_band(rate: i128, bps: u32) -> Result<(i128, i128), StrategyError> {
    let lo = math::apply_bps_ceil(rate, -(bps as i128))?;
    let hi = math::apply_bps(rate, bps as i128)?;
    Ok((lo, hi))
}

/// Refuses a fill price outside `price_band(rate, otc_max_deviation_bps)`.
fn check_price(rate: i128, price: i128, params: &Params) -> Result<(), StrategyError> {
    let (lo, hi) = price_band(rate, params.otc_max_deviation_bps)?;
    if price < lo || price > hi {
        return Err(StrategyError::InvalidArgument);
    }
    Ok(())
}

/// Buys `qty_for(usdc_amount, price)` RWA (floor) from `counterparty` for `usdc_amount`.
/// Needs `pricing::can_buy` (`Live`, positive, oracle not below the clamped rate), and `price`
/// inside the band around that rate.
pub fn invest(
    e: &Env,
    config: &Config,
    params: &Params,
    counterparty: &Address,
    usdc_amount: i128,
    price: i128,
) -> Result<i128, StrategyError> {
    if !storage::is_otc_counterparty(e, counterparty) || params.paused {
        return Err(StrategyError::NotAuthorized);
    }
    let held = pricing::rwa_balance(e, config);
    let nav = pricing::nav_guarded(e, config, params)?;
    if !pricing::can_buy(&nav.state) {
        return Err(StrategyError::ExternalError);
    }
    check_price(nav.state.rate, price, params)?;
    if usdc_amount > nav.idle {
        return Err(StrategyError::InsufficientBalance);
    }
    if usdc_amount > flows::invested_room(e, config, params, nav.state.rate, held)? {
        return Err(StrategyError::InvalidArgument);
    }
    let rwa_qty = math::qty_for(e, usdc_amount, price, config.rwa_decimals)?;
    if rwa_qty <= 0 {
        return Err(StrategyError::InvalidArgument);
    }
    let this = e.current_contract_address();
    TokenClient::new(e, &config.rwa_asset).transfer_from(&this, counterparty, &this, &rwa_qty);
    TokenClient::new(e, &config.asset).transfer(&this, counterparty, &usdc_amount);
    storage::set_expected_rwa(e, held.checked_add(rwa_qty).ok_or(StrategyError::UnderflowOverflow)?);
    OtcInvestEvent {
        strategy_name: name(e),
        counterparty: counterparty.clone(),
        usdc_in: usdc_amount,
        rwa_out: rwa_qty,
        price,
        rate: nav.state.rate,
    }
    .publish(e);
    Ok(rwa_qty)
}

/// Sells `qty_for(usdc_amount, price)` RWA (floor, one raw unit in the strategy's favor) to
/// `counterparty` for `usdc_amount`. Any mode with a rate, `price` inside the band around it.
/// Refuses rather than clamps when the RWA leg exceeds held, so the counterparty is never
/// short-paid.
pub fn divest(
    e: &Env,
    config: &Config,
    params: &Params,
    counterparty: &Address,
    usdc_amount: i128,
    price: i128,
) -> Result<i128, StrategyError> {
    if !storage::is_otc_counterparty(e, counterparty) {
        return Err(StrategyError::NotAuthorized);
    }
    let held = pricing::rwa_balance(e, config);
    let nav = pricing::nav_guarded(e, config, params)?;
    if nav.state.rate <= 0 {
        return Err(StrategyError::ExternalError);
    }
    check_price(nav.state.rate, price, params)?;
    let rwa_qty = math::qty_for(e, usdc_amount, price, config.rwa_decimals)?;
    if rwa_qty <= 0 || rwa_qty > held {
        return Err(StrategyError::InsufficientBalance);
    }
    let this = e.current_contract_address();
    TokenClient::new(e, &config.asset).transfer_from(&this, counterparty, &this, &usdc_amount);
    TokenClient::new(e, &config.rwa_asset).transfer(&this, counterparty, &rwa_qty);
    storage::set_expected_rwa(e, held.checked_sub(rwa_qty).ok_or(StrategyError::UnderflowOverflow)?);
    OtcDivestEvent {
        strategy_name: name(e),
        counterparty: counterparty.clone(),
        rwa_in: rwa_qty,
        usdc_out: usdc_amount,
        price,
        rate: nav.state.rate,
    }
    .publish(e);
    Ok(rwa_qty)
}

#[cfg(test)]
mod tests {
    use super::price_band;

    #[test]
    fn price_band_rounds_both_edges_inward() {
        // 1_000_001 * 0.995 = 995_000.995 -> ceil 995_001; * 1.005 = 1_005_001.005 -> floor
        assert_eq!(price_band(1_000_001, 50).unwrap(), (995_001, 1_005_001));
        assert_eq!(price_band(1_000_001, 0).unwrap(), (1_000_001, 1_000_001));
    }
}
