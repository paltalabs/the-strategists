//! Strategy state: an immutable `Config` (tokens, cached decimals, the venue) and a mutable
//! `Params` blob (roles, oracle set, knobs) written wholesale by the admin/keeper setters so
//! every change is one atomic instance write. Per-vault shares, the allowlists and the
//! pending counterparty proposals live in persistent storage; everything else is instance
//! storage. The venue has no setter by design.

use defindex_strategy_core::StrategyError;
use soroban_sdk::{contracttype, Address, BytesN, Env, Vec};

/// One price source. Tuple variants because `#[contracttype]` enums cannot carry named
/// fields (soroban-sdk-macros `derive_enum.rs`).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OracleKind {
    /// SEP-40 feed (Reflector-shaped): `lastprice(Asset::Stellar(asset))`, timestamps in
    /// seconds. Fields: `(feed, asset)`.
    Sep40(Address, Address),
    /// RedStone per-asset adapter: `read_price_data()`, timestamps in milliseconds,
    /// `U256` price. Field: `(adapter)`.
    RedStone(Address),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleSource {
    pub kind: OracleKind,
    /// Reject a price older than this many seconds. `(0, MAX_TTL_SECONDS]`.
    pub ttl_seconds: u64,
    /// Decimals the source reports prices in; must equal its live `decimals()`.
    pub decimals: u32,
}

/// The swap venue, called directly (no router).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Venue {
    /// Aqua liquidity pool. Fields: `(pool, rwa_idx, usdc_idx)` into the pool's token set.
    Aqua(Address, u32, u32),
    /// Sushi concentrated pool. Fields: `(pool, rwa_is_token0)`.
    Sushi(Address, bool),
}

/// Immutable configuration, set once at construction.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    /// Underlying asset the vault deposits/withdraws (USDC).
    pub asset: Address,
    /// Value-accruing RWA the strategy buys and holds.
    pub rwa_asset: Address,
    /// Decimals of `asset`, cached, <= MAX_DECIMALS.
    pub asset_decimals: u32,
    /// Decimals of `rwa_asset`, cached, <= MAX_DECIMALS (deJTRSY is 18).
    pub rwa_decimals: u32,
    /// The pool, fixed for the life of the contract. A pool migration is a new deployment.
    pub venue: Venue,
}

/// A proposed oracle set waiting out `TIMELOCK_SECONDS`. One at a time; a new proposal
/// replaces it, execute or cancel clears it.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingOracles {
    pub oracles: Vec<OracleSource>,
    pub executable_at: u64,
}

/// A proposed set of the four price knobs (§6, §9 Q11), executable at `executable_at`. One at
/// a time; a new proposal replaces it, execute or cancel clears it. The knobs a compromised
/// admin could compose into a profitable trade only ever *rise* through here, in public.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingParams {
    pub max_spread_bps: u32,
    pub slippage_bps: u32,
    pub max_swap: i128,
    pub otc_max_deviation_bps: u32,
    pub executable_at: u64,
}

/// Mutable parameters. Written wholesale by the setters.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Params {
    /// Multisig. Owns the bounded knobs and the three timelocked proposals (oracle set, OTC
    /// counterparty, any knob raise). Cannot name a recipient or a price anywhere.
    pub admin: Address,
    /// Two-step admin transfer target; `None` when no transfer is pending.
    pub pending_admin: Option<Address>,
    /// Bot. Runs invest / divest / OTC fills / harvest / pause.
    pub keeper: Address,
    /// 1..=MAX_ORACLES price sources. Changed only through `PendingOracles`.
    pub oracles: Vec<OracleSource>,
    /// USDC per swap: invest tranche, divest tranche, withdraw shortfall. Bounds our own
    /// impact on the pool.
    pub max_swap: i128,
    /// Cap on the USDC value of held RWA. Bounds the share of a pool we own.
    pub max_invested: i128,
    /// The furthest the rate may sit from the anchor, either side, and so the most the
    /// anchor may move per `ANCHOR_WINDOW_SECONDS` (§3.5).
    pub max_spread_bps: u32,
    /// Execution slippage applied to swap `min_out` / `max_in`.
    pub slippage_bps: u32,
    /// Blocks `deposit`, `invest` and `invest_otc`. Never `withdraw` or `divest`.
    pub paused: bool,
    /// When true, `deposit` buys one best-effort tranche (§3.2 step 7). Ships false.
    pub auto_invest: bool,
    /// While the ledger clock is before this timestamp the rate is held at the anchor whatever
    /// the oracles say (`PricingMode::Frozen`); 0 when not frozen. A keeper freeze (one-shot,
    /// see below) lasts `KEEPER_FREEZE_SECONDS`, then lapses; an admin freeze is `u64::MAX`, lasts
    /// until the admin lifts it. The stop for a wrong oracle during the 72 h an oracle
    /// replacement waits (§3.7).
    pub frozen_until: u64,
    /// True once the keeper has used its one freeze. Cleared only by the admin, through an
    /// unfreeze or `set_keeper`, so a compromised keeper cannot chain freezes past
    /// `KEEPER_FREEZE_SECONDS` without the multisig acting.
    pub keeper_freeze_spent: bool,
    /// OTC price band (§3.8): the keeper's per-fill `price` on `invest_otc` / `divest_otc`
    /// must sit within `rate * (1 ± otc_max_deviation_bps)`. Ships 0 (fills at exactly the
    /// rate). At most `MAX_OTC_DEVIATION_BPS`; widened only through `PendingParams`, narrowed
    /// immediately.
    pub otc_max_deviation_bps: u32,
}

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    Config,
    Params,
    TotalShares,
    /// Last `Live`-mode rate (§2.3 scale). The rate in `OracleDown` and `Frozen`, and the
    /// centre of the `Live` clamp. Written only in `Live` mode (§2.2), at most once per
    /// `ANCHOR_WINDOW_SECONDS`, moving at most `max_spread_bps` per write.
    LastValidPrice,
    /// Ledger timestamp of the last `LastValidPrice` write.
    AnchorTs,
    /// The pending oracle-set proposal, if any (`PendingOracles`).
    PendingOracles,
    /// The pending price-knob proposal, if any (`PendingParams`).
    PendingParams,
    /// RWA balance implied by accounted flows. A live balance below it means issuer action
    /// (clawback / freeze-burn); alert only.
    ExpectedRwa,
    /// Persistent. Shares held by a vault.
    VaultPos(Address),
    /// Persistent. Deposit allowlist by exact address.
    Depositor(Address),
    /// Persistent. Deposit allowlist by wasm code hash.
    AllowedHash(BytesN<32>),
    /// Persistent. Counterparties the keeper may fill OTC trades with (§3.8). Added only
    /// through `PendingOtcCounterparty`.
    OtcCounterparty(Address),
    /// Persistent. `executable_at` of a proposed counterparty.
    PendingOtcCounterparty(Address),
    LastReanchorTs,
}

pub const ONE_DAY_LEDGERS: u32 = 17_280;
pub const INSTANCE_BUMP_AMOUNT: u32 = 30 * ONE_DAY_LEDGERS;
pub const INSTANCE_LIFETIME_THRESHOLD: u32 = INSTANCE_BUMP_AMOUNT - ONE_DAY_LEDGERS;
const PERSISTENT_BUMP_AMOUNT: u32 = 120 * ONE_DAY_LEDGERS;
const PERSISTENT_LIFETIME_THRESHOLD: u32 = PERSISTENT_BUMP_AMOUNT - 20 * ONE_DAY_LEDGERS;

/// Bumps the instance storage TTL (`Config`, `Params` and every other instance-tier key
/// live under one lifetime). Call on every entry point that touches storage.
pub fn extend_instance_ttl(e: &Env) {
    e.storage()
        .instance()
        .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

fn bump_persistent(e: &Env, key: &DataKey) {
    e.storage()
        .persistent()
        .extend_ttl(key, PERSISTENT_LIFETIME_THRESHOLD, PERSISTENT_BUMP_AMOUNT);
}

// --- Config / Params -------------------------------------------------------

/// Instance storage. Writes `Config` wholesale; set once at construction.
pub fn set_config(e: &Env, config: &Config) {
    e.storage().instance().set(&DataKey::Config, config);
}

/// Instance storage. Reads `Config`, or `NotInitialized` before construction.
pub fn get_config(e: &Env) -> Result<Config, StrategyError> {
    e.storage().instance().get(&DataKey::Config).ok_or(StrategyError::NotInitialized)
}

/// Instance storage. Writes `Params` wholesale; every setter reads-modifies-writes the
/// whole blob so a change is one atomic instance write.
pub fn set_params(e: &Env, params: &Params) {
    e.storage().instance().set(&DataKey::Params, params);
}

/// Instance storage. Reads `Params`, or `NotInitialized` before construction.
pub fn get_params(e: &Env) -> Result<Params, StrategyError> {
    e.storage().instance().get(&DataKey::Params).ok_or(StrategyError::NotInitialized)
}

// --- Shares ----------------------------------------------------------------

/// Instance storage. Writes the strategy's total outstanding shares.
pub fn set_total_shares(e: &Env, total: i128) {
    e.storage().instance().set(&DataKey::TotalShares, &total);
}

/// Instance storage. Reads total outstanding shares, defaulting to 0 (`unwrap_or(0)`)
/// before any deposit has minted shares.
pub fn get_total_shares(e: &Env) -> i128 {
    e.storage().instance().get(&DataKey::TotalShares).unwrap_or(0)
}

/// A zero balance removes the entry so a fully-withdrawn vault leaves no rent-paying dust.
pub fn set_vault_shares(e: &Env, address: &Address, shares: i128) {
    let key = DataKey::VaultPos(address.clone());
    if shares == 0 {
        e.storage().persistent().remove(&key);
        return;
    }
    e.storage().persistent().set::<DataKey, i128>(&key, &shares);
    bump_persistent(e, &key);
}

/// Read with a TTL bump: for state-changing paths.
pub fn get_vault_shares(e: &Env, address: &Address) -> i128 {
    let key = DataKey::VaultPos(address.clone());
    match e.storage().persistent().get::<DataKey, i128>(&key) {
        Some(shares) => {
            bump_persistent(e, &key);
            shares
        }
        None => 0,
    }
}

/// Read without any TTL bump: for `balance()`, which must stay a pure read (§3.6).
pub fn peek_vault_shares(e: &Env, address: &Address) -> i128 {
    e.storage()
        .persistent()
        .get::<DataKey, i128>(&DataKey::VaultPos(address.clone()))
        .unwrap_or(0)
}

// --- Tripwires -------------------------------------------------------------

/// Writes the anchor and its timestamp together; `nav_guarded` and `reanchor` are the only callers.
pub fn set_anchor(e: &Env, rate: i128, ts: u64) {
    e.storage().instance().set(&DataKey::LastValidPrice, &rate);
    e.storage().instance().set(&DataKey::AnchorTs, &ts);
}

/// Instance storage. Reads the `LastValidPrice` anchor rate, `None` before the first
/// `Live`-mode write.
pub fn get_last_valid_price(e: &Env) -> Option<i128> {
    e.storage().instance().get(&DataKey::LastValidPrice)
}

/// Instance storage. Reads the ledger timestamp of the last anchor write, `None` before
/// the first `Live`-mode write.
pub fn get_anchor_ts(e: &Env) -> Option<u64> {
    e.storage().instance().get(&DataKey::AnchorTs)
}

// --- Timelocked proposals ----------------------------------------------------

/// Instance storage. Writes the proposed oracle set wholesale; a new proposal replaces
/// any existing one.
pub fn set_pending_oracles(e: &Env, pending: &PendingOracles) {
    e.storage().instance().set(&DataKey::PendingOracles, pending);
}

/// Instance storage. Reads the pending oracle-set proposal, `None` when none is pending.
pub fn get_pending_oracles(e: &Env) -> Option<PendingOracles> {
    e.storage().instance().get(&DataKey::PendingOracles)
}

/// Instance storage. Removes the pending oracle-set proposal (execute or cancel).
pub fn clear_pending_oracles(e: &Env) {
    e.storage().instance().remove(&DataKey::PendingOracles);
}

/// Instance storage. Writes the proposed price knobs wholesale; a new proposal replaces
/// any existing one.
pub fn set_pending_params(e: &Env, pending: &PendingParams) {
    e.storage().instance().set(&DataKey::PendingParams, pending);
}

/// Instance storage. Reads the pending price-knob proposal, `None` when none is pending.
pub fn get_pending_params(e: &Env) -> Option<PendingParams> {
    e.storage().instance().get(&DataKey::PendingParams)
}

/// Instance storage. Removes the pending price-knob proposal (execute or cancel).
pub fn clear_pending_params(e: &Env) {
    e.storage().instance().remove(&DataKey::PendingParams);
}

/// Persistent storage. Writes a proposed counterparty's `executable_at` timelock and
/// bumps its TTL.
pub fn set_pending_otc_counterparty(e: &Env, address: &Address, executable_at: u64) {
    let key = DataKey::PendingOtcCounterparty(address.clone());
    e.storage().persistent().set(&key, &executable_at);
    bump_persistent(e, &key);
}

/// Persistent storage. Reads a proposed counterparty's `executable_at`, `None` when
/// nothing is pending for that address. Does not bump the TTL.
pub fn get_pending_otc_counterparty(e: &Env, address: &Address) -> Option<u64> {
    e.storage().persistent().get(&DataKey::PendingOtcCounterparty(address.clone()))
}

/// Persistent storage. Removes a proposed counterparty's timelock (execute or cancel).
pub fn clear_pending_otc_counterparty(e: &Env, address: &Address) {
    e.storage().persistent().remove(&DataKey::PendingOtcCounterparty(address.clone()));
}

/// Instance storage. Writes the RWA balance implied by accounted flows (the clawback /
/// freeze-burn tripwire).
pub fn set_expected_rwa(e: &Env, balance: i128) {
    e.storage().instance().set(&DataKey::ExpectedRwa, &balance);
}

/// Instance storage. Reads the expected RWA balance, `None` before it is first set.
pub fn get_expected_rwa(e: &Env) -> Option<i128> {
    e.storage().instance().get(&DataKey::ExpectedRwa)
}

// --- Allowlists (persistent bool flags; revoking removes the entry) ----------

fn set_flag(e: &Env, key: DataKey, allowed: bool) {
    if allowed {
        e.storage().persistent().set::<DataKey, bool>(&key, &true);
        bump_persistent(e, &key);
    } else {
        e.storage().persistent().remove(&key);
    }
}

fn get_flag(e: &Env, key: DataKey) -> bool {
    match e.storage().persistent().get::<DataKey, bool>(&key) {
        Some(v) => {
            bump_persistent(e, &key);
            v
        }
        None => false,
    }
}

/// Persistent storage. Sets or revokes an exact-address depositor allowlist entry:
/// `true` writes the flag and bumps its TTL, `false` removes the entry outright.
pub fn set_depositor(e: &Env, address: &Address, allowed: bool) {
    set_flag(e, DataKey::Depositor(address.clone()), allowed);
}

/// Persistent storage. Reads the exact-address depositor allowlist flag, `false` when
/// absent; bumps the TTL when the entry is present.
pub fn is_depositor(e: &Env, address: &Address) -> bool {
    get_flag(e, DataKey::Depositor(address.clone()))
}

/// Persistent storage. Sets or revokes a wasm-code-hash depositor allowlist entry:
/// `true` writes the flag and bumps its TTL, `false` removes the entry outright.
pub fn set_allowed_hash(e: &Env, hash: &BytesN<32>, allowed: bool) {
    set_flag(e, DataKey::AllowedHash(hash.clone()), allowed);
}

/// Persistent storage. Reads the wasm-code-hash depositor allowlist flag, `false` when
/// absent; bumps the TTL when the entry is present.
pub fn is_allowed_hash(e: &Env, hash: &BytesN<32>) -> bool {
    get_flag(e, DataKey::AllowedHash(hash.clone()))
}

/// Persistent storage. Sets or revokes an OTC counterparty allowlist entry: `true`
/// writes the flag and bumps its TTL, `false` removes the entry outright.
pub fn set_otc_counterparty(e: &Env, address: &Address, allowed: bool) {
    set_flag(e, DataKey::OtcCounterparty(address.clone()), allowed);
}

/// Persistent storage. Reads the OTC counterparty allowlist flag, `false` when absent;
/// bumps the TTL when the entry is present.
pub fn is_otc_counterparty(e: &Env, address: &Address) -> bool {
    get_flag(e, DataKey::OtcCounterparty(address.clone()))
}

// --- Reanchor cooldown -----------------------------------------------------

/// Ledger timestamp of the last `reanchor_to_oracle`. Kept apart from `AnchorTs`, which every
/// guarded `Live` write moves, so keeper calls cannot keep the cooldown from expiring.
pub fn set_last_reanchor_ts(e: &Env, ts: u64) {
    e.storage().instance().set(&DataKey::LastReanchorTs, &ts);
}

/// Instance storage. `None` until the first `reanchor_to_oracle`.
pub fn get_last_reanchor_ts(e: &Env) -> Option<u64> {
    e.storage().instance().get(&DataKey::LastReanchorTs)
}
