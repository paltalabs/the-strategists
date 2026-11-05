//! the changes that add trust or widen a price bound — a new oracle set, a new
//! OTC counterparty, a *raise* of the four price knobs — are proposed, announced by event, and
//! executable only `TIMELOCK_SECONDS` later. Removing trust (`remove_counterparty`,
//! `cancel_oracles`) and narrowing a bound (the immediate setters in `lib.rs`) is immediate. The
//! wait turns "a price the admin chose" into "a price the public saw first"; the
//! `Proposed` events are what the bot and the vault manager act on inside the window. Entry points in `lib.rs` do auth and TTL; everything else lives here.

use crate::constants::{
    MAX_OTC_DEVIATION_BPS, MAX_SLIPPAGE_BPS, MAX_SPREAD_BPS, MIN_SPREAD_BPS, STRATEGY_NAME, TIMELOCK_SECONDS,
};
use crate::events::{
    emit_params_update, OraclesProposalCancelledEvent, OraclesProposedEvent, OraclesSetEvent,
    OtcCounterpartyProposedEvent, OtcCounterpartySetEvent, ParamsProposalCancelledEvent, ParamsProposedEvent,
};
use crate::oracles;
use crate::storage::{self, Config, OracleSource, Params, PendingOracles, PendingParams};
use defindex_strategy_core::StrategyError;
use soroban_sdk::{Address, Env, String, Vec};

fn name(e: &Env) -> String {
    String::from_str(e, STRATEGY_NAME)
}

fn executable_at(e: &Env) -> Result<u64, StrategyError> {
    e.ledger().timestamp().checked_add(TIMELOCK_SECONDS).ok_or(StrategyError::UnderflowOverflow)
}

/// Validates like the constructor (count, TTL, decimals, live reachability), stores the
/// proposal replacing any earlier one, emits `OraclesProposedEvent`. Returns `executable_at`.
pub fn propose_oracles(e: &Env, oracles: Vec<OracleSource>) -> Result<u64, StrategyError> {
    oracles::validate_oracles(e, &oracles)?;
    let at = executable_at(e)?;
    let count = oracles.len();
    storage::set_pending_oracles(e, &PendingOracles { oracles, executable_at: at });
    OraclesProposedEvent { strategy_name: name(e), count, executable_at: at }.publish(e);
    Ok(at)
}

/// Applies the pending set once its time has come and `market_check` passes. Writes
/// `params.oracles`, clears the proposal, emits `OraclesSetEvent`. Returns the new count.
/// A failed market check leaves both the old set and the proposal in place.
pub fn execute_oracles(e: &Env, config: &Config, params: &Params) -> Result<u32, StrategyError> {
    let pending = storage::get_pending_oracles(e).ok_or(StrategyError::InvalidArgument)?;
    if e.ledger().timestamp() < pending.executable_at {
        return Err(StrategyError::NotAuthorized);
    }
    oracles::market_check(e, config, &pending.oracles)?;
    let mut updated = params.clone();
    updated.oracles = pending.oracles;
    let count = updated.oracles.len();
    storage::set_params(e, &updated);
    storage::clear_pending_oracles(e);
    OraclesSetEvent { strategy_name: name(e), count }.publish(e);
    Ok(count)
}

/// Drops the pending proposal. Refuses when there is none.
pub fn cancel_oracles(e: &Env) -> Result<(), StrategyError> {
    if storage::get_pending_oracles(e).is_none() {
        return Err(StrategyError::InvalidArgument);
    }
    storage::clear_pending_oracles(e);
    OraclesProposalCancelledEvent { strategy_name: name(e) }.publish(e);
    Ok(())
}

/// Validates the four knobs like the constructor plus the OTC band cap, stores the proposal
/// replacing any earlier one, emits `ParamsProposedEvent`. Returns `executable_at`. Any value
/// may be proposed, lower ones included: the wait is what makes a raise public.
pub fn propose_params(
    e: &Env,
    max_spread_bps: u32,
    slippage_bps: u32,
    max_swap: i128,
    otc_max_deviation_bps: u32,
) -> Result<u64, StrategyError> {
    if !(MIN_SPREAD_BPS..=MAX_SPREAD_BPS).contains(&max_spread_bps)
        || slippage_bps > MAX_SLIPPAGE_BPS
        || max_swap <= 0
        || otc_max_deviation_bps > MAX_OTC_DEVIATION_BPS
    {
        return Err(StrategyError::InvalidArgument);
    }
    let at = executable_at(e)?;
    storage::set_pending_params(
        e,
        &PendingParams { max_spread_bps, slippage_bps, max_swap, otc_max_deviation_bps, executable_at: at },
    );
    ParamsProposedEvent {
        strategy_name: name(e),
        max_spread_bps,
        slippage_bps,
        max_swap,
        otc_max_deviation_bps,
        executable_at: at,
    }
    .publish(e);
    Ok(at)
}

/// Applies the pending knobs once their time has come. No market check: these are bounded
/// constants, not a price. Writes the four fields, clears the proposal, emits one
/// `ParamsUpdatedEvent` per field whose value changed.
pub fn execute_params(e: &Env, params: &Params) -> Result<(), StrategyError> {
    let pending = storage::get_pending_params(e).ok_or(StrategyError::InvalidArgument)?;
    if e.ledger().timestamp() < pending.executable_at {
        return Err(StrategyError::NotAuthorized);
    }
    let mut updated = params.clone();
    updated.max_spread_bps = pending.max_spread_bps;
    updated.slippage_bps = pending.slippage_bps;
    updated.max_swap = pending.max_swap;
    updated.otc_max_deviation_bps = pending.otc_max_deviation_bps;
    storage::set_params(e, &updated);
    storage::clear_pending_params(e);
    emit_if_changed(e, "max_spread_bps", params.max_spread_bps as i128, pending.max_spread_bps as i128);
    emit_if_changed(e, "slippage_bps", params.slippage_bps as i128, pending.slippage_bps as i128);
    emit_if_changed(e, "max_swap", params.max_swap, pending.max_swap);
    emit_if_changed(e, "otc_max_deviation_bps", params.otc_max_deviation_bps as i128, pending.otc_max_deviation_bps as i128);
    Ok(())
}

/// Drops the pending proposal. Refuses when there is none.
pub fn cancel_params(e: &Env) -> Result<(), StrategyError> {
    if storage::get_pending_params(e).is_none() {
        return Err(StrategyError::InvalidArgument);
    }
    storage::clear_pending_params(e);
    ParamsProposalCancelledEvent { strategy_name: name(e) }.publish(e);
    Ok(())
}

/// One `ParamsUpdatedEvent` only when the value moved. `execute_params` writes four fields at
/// once, and a watcher should see exactly the ones that changed — unlike the immediate setters,
/// which emit per field unconditionally because the caller named every one of them.
fn emit_if_changed(e: &Env, field: &str, old_value: i128, new_value: i128) {
    if old_value != new_value {
        emit_params_update(e, field, old_value, new_value);
    }
}

/// Announces a counterparty; it can be executed `TIMELOCK_SECONDS` later. Returns that time.
pub fn propose_counterparty(e: &Env, counterparty: &Address) -> Result<u64, StrategyError> {
    let at = executable_at(e)?;
    storage::set_pending_otc_counterparty(e, counterparty, at);
    OtcCounterpartyProposedEvent { strategy_name: name(e), counterparty: counterparty.clone(), executable_at: at }
        .publish(e);
    Ok(at)
}

/// Allowlists a proposed counterparty once its time has come.
pub fn execute_counterparty(e: &Env, counterparty: &Address) -> Result<(), StrategyError> {
    let at = storage::get_pending_otc_counterparty(e, counterparty).ok_or(StrategyError::InvalidArgument)?;
    if e.ledger().timestamp() < at {
        return Err(StrategyError::NotAuthorized);
    }
    storage::set_otc_counterparty(e, counterparty, true);
    storage::clear_pending_otc_counterparty(e, counterparty);
    OtcCounterpartySetEvent { strategy_name: name(e), counterparty: counterparty.clone(), allowed: true }.publish(e);
    Ok(())
}

/// Immediate: removing trust never waits. Clears the allowlist entry and any pending
/// proposal for the address.
pub fn remove_counterparty(e: &Env, counterparty: &Address) {
    storage::set_otc_counterparty(e, counterparty, false);
    storage::clear_pending_otc_counterparty(e, counterparty);
    OtcCounterpartySetEvent { strategy_name: name(e), counterparty: counterparty.clone(), allowed: false }.publish(e);
}
