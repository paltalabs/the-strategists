//! Every locally defined event. `DepositEvent`, `WithdrawEvent` and `HarvestEvent` come from
//! `defindex_strategy_core::event`.

use crate::constants::STRATEGY_NAME;
use crate::pricing::PricingMode;
use soroban_sdk::{contractevent, Address, BytesN, Env, String, Symbol};

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetKeeperEvent {
    #[topic]
    pub strategy_name: String,
    pub old_keeper: Address,
    pub new_keeper: Address,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminTransferStartedEvent {
    #[topic]
    pub strategy_name: String,
    pub admin: Address,
    pub pending_admin: Address,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminTransferredEvent {
    #[topic]
    pub strategy_name: String,
    pub old_admin: Address,
    pub new_admin: Address,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetDepositorEvent {
    #[topic]
    pub strategy_name: String,
    pub depositor: Address,
    pub allowed: bool,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetAllowedHashEvent {
    #[topic]
    pub strategy_name: String,
    pub hash: BytesN<32>,
    pub allowed: bool,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OtcCounterpartySetEvent {
    #[topic]
    pub strategy_name: String,
    pub counterparty: Address,
    pub allowed: bool,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PauseEvent {
    #[topic]
    pub strategy_name: String,
    pub paused: bool,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutoInvestSetEvent {
    #[topic]
    pub strategy_name: String,
    pub enabled: bool,
}

/// Old and new value of one field — `max_swap`, `max_invested`, `max_spread_bps`,
/// `slippage_bps` or `otc_max_deviation_bps` — named by `field`; see `emit_params_update`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParamsUpdatedEvent {
    #[topic]
    pub strategy_name: String,
    #[topic]
    pub field: Symbol,
    pub old_value: i128,
    pub new_value: i128,
}

/// Publishes one `ParamsUpdatedEvent` for `field` unconditionally — this function never checks
/// whether `old_value` and `new_value` differ. The immediate setters in `lib.rs` call it once
/// per field they named, whether or not the value moved, because the caller typed every one of
/// them. The changed-only gating a watcher relies on for `execute_params` lives one level up, in
/// `timelock::emit_if_changed` (`src/timelock.rs:137`), the only caller that filters before
/// calling here.
pub fn emit_params_update(e: &Env, field: &str, old_value: i128, new_value: i128) {
    ParamsUpdatedEvent {
        strategy_name: String::from_str(e, STRATEGY_NAME),
        field: Symbol::new(e, field),
        old_value,
        new_value,
    }
    .publish(e);
}

/// A new oracle set was proposed; executable at `executable_at`. This is the public notice
/// the bot and the vault manager watch for (spec §8).
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OraclesProposedEvent {
    #[topic]
    pub strategy_name: String,
    pub count: u32,
    pub executable_at: u64,
}

/// The proposed oracle set was executed and is now live.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OraclesSetEvent {
    #[topic]
    pub strategy_name: String,
    pub count: u32,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OraclesProposalCancelledEvent {
    #[topic]
    pub strategy_name: String,
}

/// New values for the four price knobs were proposed; executable at `executable_at`. Carries
/// every proposed value, so the 72 h notice says exactly what is about to change (§9 Q11).
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParamsProposedEvent {
    #[topic]
    pub strategy_name: String,
    pub max_spread_bps: u32,
    pub slippage_bps: u32,
    pub max_swap: i128,
    pub otc_max_deviation_bps: u32,
    pub executable_at: u64,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParamsProposalCancelledEvent {
    #[topic]
    pub strategy_name: String,
}

/// A counterparty was proposed; it can trade from `executable_at` once executed.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OtcCounterpartyProposedEvent {
    #[topic]
    pub strategy_name: String,
    pub counterparty: Address,
    pub executable_at: u64,
}

/// Harvest could not price in `Live` mode (no live oracle, or the price is frozen). Telemetry only.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpreadAlertEvent {
    #[topic]
    pub strategy_name: String,
    pub timestamp: u64,
}

/// Live RWA balance below what accounted flows imply: issuer clawback or freeze-burn.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RwaBalanceAnomalyEvent {
    #[topic]
    pub strategy_name: String,
    pub expected: i128,
    pub actual: i128,
}

/// Emitted by every state-changing pricing outside `Live` mode.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PricingModeEvent {
    #[topic]
    pub strategy_name: String,
    pub mode: PricingMode,
    pub live_oracles: u32,
    pub rate: i128,
}

/// The raw `Live` rate is more than `max_spread_bps` from the anchor; NAV is booked at
/// the clamped rate until the anchor catches up, one spread per `ANCHOR_WINDOW_SECONDS`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceJumpEvent {
    #[topic]
    pub strategy_name: String,
    pub last_rate: i128,
    pub new_rate: i128,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvestEvent {
    #[topic]
    pub strategy_name: String,
    pub usdc_in: i128,
    pub rwa_out: i128,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DivestEvent {
    #[topic]
    pub strategy_name: String,
    pub rwa_in: i128,
    pub usdc_out: i128,
}

/// An OTC buy. `price` is what the keeper passed and the fill paid; `rate` is the gated rate
/// it was checked against, so a monitor reads every fill's distance from NAV off the event.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OtcInvestEvent {
    #[topic]
    pub strategy_name: String,
    pub counterparty: Address,
    pub usdc_in: i128,
    pub rwa_out: i128,
    pub price: i128,
    pub rate: i128,
}

/// An OTC sell. `price` and `rate` as on `OtcInvestEvent`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OtcDivestEvent {
    #[topic]
    pub strategy_name: String,
    pub counterparty: Address,
    pub rwa_in: i128,
    pub usdc_out: i128,
    pub price: i128,
    pub rate: i128,
}

/// `freeze_price` changed `Params.frozen_until`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceFrozenEvent {
    #[topic]
    pub strategy_name: String,
    pub caller: Address,
    pub frozen: bool,
    /// The new `Params.frozen_until`.
    pub frozen_until: u64,
}

/// `reanchor_to_oracle` lowered the anchor to the oracle rate (Halborn FIND-002).
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReanchorEvent {
    #[topic]
    pub strategy_name: String,
    pub old_rate: i128,
    pub new_rate: i128,
    /// The admin's confirmed figure the oracle had to match.
    pub expected_rate: i128,
}
