//! Oracle-priced. Inputs: the live oracle rates `O` (each `try_`) and the
//! anchor `L = LastValidPrice`. The venue is not an input: the USDY and deJTRSY pools trade
//! 120-150 bps under issuer NAV for weeks at a time
//! (see `docs/strategies/rwa-yield-sharing/economic-design.md`), so pricing off them would book every OTC fill at NAV as a loss. The pools are only
//! where swaps execute, and every swap is bounded by `slippage_bps` around this rate.
//!
//! Output: `min(O)` clamped to `[L*(1-s), L*(1+s)]`, `s = max_spread_bps`. `L` is written
//! from that clamped rate at most once per `ANCHOR_WINDOW_SECONDS`, so no oracle, however
//! wrong, moves the rate more than one `s` per window. With no live oracle, or with the
//! price frozen by the admin or keeper, the rate is `L`. Never an error for an external
//! reason: that is what keeps the vault's `balance()` call alive.

use crate::constants::{ANCHOR_WINDOW_SECONDS, MAX_RATE, MAX_REANCHOR_STEP_BPS, STRATEGY_NAME};
use crate::events::{PriceJumpEvent, PricingModeEvent};
use crate::math;
use crate::oracles;
use crate::storage::{self, Config, Params};
use defindex_strategy_core::StrategyError;
use soroban_sdk::{contracttype, token::TokenClient, Env, String};

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PricingMode {
    /// At least one oracle live: `min(O)` clamped to `[L*(1-s), L*(1+s)]` (raw without an
    /// anchor). The only mode that writes the anchor, buys RWA, or mints against held RWA.
    Live,
    /// No live oracle: `L`, or 0 with no anchor.
    OracleDown,
    /// Frozen by `freeze_price` until `Params.frozen_until`: `L`, or 0 with no anchor, whatever
    /// the oracles say.
    Frozen,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PricingState {
    /// The bounded rate every flow and `balance()` use.
    pub rate: i128,
    /// `min(O)` before the anchor clamp in `Live`; equals `rate` in the other modes.
    pub raw_rate: i128,
    pub mode: PricingMode,
    pub live_oracles: u32,
}

/// One priced snapshot for a flow. No held-RWA field: every production caller already has
/// that quantity from `lib::check_rwa_balance_anomaly`, whose alert and resync it cannot replace.
pub struct Nav {
    pub nav: i128,
    pub idle: i128,
    pub state: PricingState,
}

/// Raw RWA units the strategy holds.
pub fn rwa_balance(e: &Env, config: &Config) -> i128 {
    TokenClient::new(e, &config.rwa_asset).balance(&e.current_contract_address())
}

/// USDC stroops sitting idle in the strategy.
pub fn idle_usdc(e: &Env, config: &Config) -> i128 {
    TokenClient::new(e, &config.asset).balance(&e.current_contract_address())
}

/// `(min of the live oracle rates, how many were live)`. `None` when none is live.
fn min_live_oracle(e: &Env, config: &Config, params: &Params) -> (Option<i128>, u32) {
    let mut min: Option<i128> = None;
    let mut live = 0u32;
    for source in params.oracles.iter() {
        if let Some(rate) = oracles::read_rate(e, config, &source) {
            live += 1;
            min = Some(min.map_or(rate, |m: i128| m.min(rate)));
        }
    }
    (min, live)
}

/// True when a buy may run at `state.rate`: `Live`, positive, and the oracle not reading below
/// the clamped rate. When `raw_rate < rate` the clamp is holding NAV above what the oracle says,
/// so buying at `rate` would pay more than the oracle's price for the RWA.
pub fn can_buy(state: &PricingState) -> bool {
    state.mode == PricingMode::Live && state.rate > 0 && state.raw_rate >= state.rate
}

/// The rate. Pure read (reads `LastValidPrice`, writes nothing).
pub fn price_rwa(e: &Env, config: &Config, params: &Params) -> Result<PricingState, StrategyError> {
    let (min_o, live_oracles) = min_live_oracle(e, config, params);
    let anchor = storage::get_last_valid_price(e);
    let spread = params.max_spread_bps as i128;

    let frozen = e.ledger().timestamp() < params.frozen_until;
    let (raw_rate, rate, mode) = match (frozen, min_o) {
        (true, _) => {
            let r = anchor.unwrap_or(0);
            (r, r, PricingMode::Frozen)
        }
        (false, Some(raw)) => {
            let clamped = match anchor {
                Some(l) => raw.max(math::apply_bps(l, -spread)?).min(math::apply_bps(l, spread)?),
                None => raw,
            };
            (raw, clamped, PricingMode::Live)
        }
        (false, None) => {
            let r = anchor.unwrap_or(0);
            (r, r, PricingMode::OracleDown)
        }
    };
    Ok(PricingState { rate, raw_rate, mode, live_oracles })
}

/// NAV = idle USDC + held RWA at the rate. Pure read: for `balance()`.
pub fn nav_readonly(e: &Env, config: &Config, params: &Params) -> Result<Nav, StrategyError> {
    let held = rwa_balance(e, config);
    let idle = idle_usdc(e, config);
    let state = price_rwa(e, config, params)?;
    let nav = idle
        .checked_add(math::value_of(e, held, state.rate, config.rwa_decimals)?)
        .ok_or(StrategyError::ArithmeticError)?;
    Ok(Nav { nav, idle, state })
}

/// Same NAV, plus the writes and events state-changing paths need: the anchor is written
/// only in `Live` and only when `ANCHOR_WINDOW_SECONDS` have passed since the last
/// write (or none exists), so it moves at most one spread per window; `PricingModeEvent`
/// otherwise; `PriceJumpEvent` when the raw `Live` rate is more than the spread from the
/// anchor (NAV is booked at the clamped rate meanwhile).
pub fn nav_guarded(e: &Env, config: &Config, params: &Params) -> Result<Nav, StrategyError> {
    let n = nav_readonly(e, config, params)?;
    let name = String::from_str(e, STRATEGY_NAME);
    if n.state.mode == PricingMode::Live {
        let now = e.ledger().timestamp();
        match (storage::get_last_valid_price(e), storage::get_anchor_ts(e)) {
            (Some(last), Some(ts)) => {
                if last > 0 && math::spread_bps(n.state.raw_rate, last)? > params.max_spread_bps as i128 {
                    PriceJumpEvent { strategy_name: name, last_rate: last, new_rate: n.state.raw_rate }.publish(e);
                }
                if now.saturating_sub(ts) >= ANCHOR_WINDOW_SECONDS {
                    storage::set_anchor(e, n.state.rate, now);
                }
            }
            _ => storage::set_anchor(e, n.state.rate, now),
        }
    } else {
        PricingModeEvent {
            strategy_name: name,
            mode: n.state.mode,
            live_oracles: n.state.live_oracles,
            rate: n.state.rate,
        }
        .publish(e);
    }
    Ok(n)
}

/// The admin's `reanchor_to_oracle`: moves the anchor down toward the oracle
/// when a real impairment would otherwise take one `max_spread_bps` step per
/// `ANCHOR_WINDOW_SECONDS` to book, during which every exit is paid at the higher NAV. Returns
/// `(old anchor, new anchor)`.
///
/// It can only lower NAV, and only toward a rate every configured source confirms: not frozen
/// (a freeze says the oracle is not trusted), no reanchor in the last window, an anchor to
/// lower, **every** source live and fresh, each within `max_spread_bps` of `expected_rate` (the
/// figure the multisig checked off-chain, so a feed that moved or broke between signing and
/// execution refuses), and their minimum, the rate `Live` pricing would use, strictly under
/// the anchor. The anchor lands on that minimum or `MAX_REANCHOR_STEP_BPS` under the old
/// anchor, whichever is higher; the step rounds up so it never exceeds the cap. So a
/// compromised multisig with a wrong-low oracle takes at most 5% per window, and a real drop
/// past 5% books over several windows. The anchor timestamp restarts, so the clamp then moves
/// at most one spread per window from the new anchor, upward included; within any window the
/// anchor falls at most one clamp write plus one reanchor.
pub fn reanchor(e: &Env, config: &Config, params: &Params, expected_rate: i128) -> Result<(i128, i128), StrategyError> {
    if expected_rate <= 0 || expected_rate > MAX_RATE {
        return Err(StrategyError::InvalidArgument);
    }
    let now = e.ledger().timestamp();
    if now < params.frozen_until {
        return Err(StrategyError::ExternalError);
    }
    if let Some(last) = storage::get_last_reanchor_ts(e) {
        if now < last.saturating_add(ANCHOR_WINDOW_SECONDS) {
            return Err(StrategyError::InvalidArgument);
        }
    }
    let anchor = storage::get_last_valid_price(e).ok_or(StrategyError::InvalidArgument)?;
    let mut min: Option<i128> = None;
    for source in params.oracles.iter() {
        let rate = oracles::read_rate(e, config, &source).ok_or(StrategyError::ExternalError)?;
        if math::spread_bps(rate, expected_rate)? > params.max_spread_bps as i128 {
            return Err(StrategyError::InvalidArgument);
        }
        min = Some(min.map_or(rate, |m: i128| m.min(rate)));
    }
    let raw = min.ok_or(StrategyError::ExternalError)?;
    if raw >= anchor {
        return Err(StrategyError::InvalidArgument);
    }
    let floor = math::apply_bps_ceil(anchor, -(MAX_REANCHOR_STEP_BPS as i128))?;
    let new_anchor = raw.max(floor);
    storage::set_anchor(e, new_anchor, now);
    storage::set_last_reanchor_ts(e, now);
    Ok((anchor, new_anchor))
}
