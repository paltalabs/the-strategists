#![no_std]
//! # RWA Yield-Sharing Strategy (USDC-in / USDC-out)
//!
//! A DeFindex strategy that holds a value-accruing RWA stablebond (USTRY, USDY, deJTRSY,
//! CETES). The vault deposits USDC; the strategy keeps it idle, buys and sells the RWA with
//! an allowlisted OTC counterparty or in capped tranches on an Aqua or Sushi pool, and pays
//! withdrawals from idle first. Yield accrues as RWA price appreciation. `harvest` is a
//! telemetry no-op.
//!
//! Design (see `docs/strategies/rwa-yield-sharing/`): `balance()` is total
//! (never reverts on an external dependency), the RWA is priced at its oracle NAV, never at
//! the pool, and that rate moves at most `max_spread_bps` per hour around an anchor, the venue is a pool called
//! directly and never changed, deposits are accepted whole and invested by tranche, and a
//! multisig admin holds bounded knobs plus three 72 h-timelocked proposals (oracle set, OTC
//! counterparty, any knob raise). No method moves tokens to a caller-chosen address. Not upgradable.

mod abi;
mod constants;
mod events;
mod flows;
mod math;
mod oracles;
mod otc;
mod pricing;
mod shares;
mod storage;
mod timelock;
mod venue;
mod venue_aqua;
mod venue_sushi;

pub use defindex_strategy_core::{event, DeFindexStrategyTrait, StrategyError};
pub use pricing::{PricingMode, PricingState};
pub use storage::{Config, OracleKind, OracleSource, Params, PendingOracles, PendingParams, Venue};

use constants::{KEEPER_FREEZE_SECONDS, MAX_OTC_DEVIATION_BPS, MAX_SLIPPAGE_BPS, MAX_SPREAD_BPS, MIN_SPREAD_BPS, STRATEGY_NAME};
use events::{
    emit_params_update, AdminTransferStartedEvent, AdminTransferredEvent, AutoInvestSetEvent, DivestEvent,
    PauseEvent, PriceFrozenEvent, ReanchorEvent, RwaBalanceAnomalyEvent, SetAllowedHashEvent,
    SetDepositorEvent, SetKeeperEvent, SpreadAlertEvent,
};
use soroban_sdk::{
    contract, contractimpl, token::TokenClient, Address, Bytes, BytesN, Env, Executable, IntoVal,
    String, Val, Vec,
};
use storage::extend_instance_ttl;

fn name(e: &Env) -> String {
    String::from_str(e, STRATEGY_NAME)
}

fn check_positive_amount(amount: i128) -> Result<(), StrategyError> {
    if amount <= 0 {
        Err(StrategyError::OnlyPositiveAmountAllowed)
    } else {
        Ok(())
    }
}

/// Either the exact address is allowlisted, or it is a contract whose wasm hash is.
fn is_authorized_depositor(e: &Env, from: &Address) -> bool {
    if storage::is_depositor(e, from) {
        return true;
    }
    match from.executable() {
        Some(Executable::Wasm(hash)) => storage::is_allowed_hash(e, &hash),
        _ => false,
    }
}

/// `params.admin.require_auth()` as one call site.
fn require_admin(params: &Params) {
    params.admin.require_auth();
}

/// Compare the live RWA balance against what accounted flows imply; alert on a shortfall
/// (issuer clawback or freeze-burn), never revert; resync. Returns the live balance.
fn check_rwa_balance_anomaly(e: &Env, config: &Config) -> i128 {
    let live = TokenClient::new(e, &config.rwa_asset).balance(&e.current_contract_address());
    if let Some(expected) = storage::get_expected_rwa(e) {
        if live < expected {
            RwaBalanceAnomalyEvent { strategy_name: name(e), expected, actual: live }.publish(e);
        }
    }
    storage::set_expected_rwa(e, live);
    live
}

#[contract]
pub struct StableBondWrapperStrategy;

#[contractimpl]
impl DeFindexStrategyTrait for StableBondWrapperStrategy {
    /// Initializes the strategy. `asset` is USDC. `init_args` (9, in order):
    /// 0. `rwa_asset: Address`
    /// 1. `admin: Address` (multisig)
    /// 2. `keeper: Address` (bot)
    /// 3. `oracles: Vec<OracleSource>` (1..=3)
    /// 4. `venue: Venue`
    /// 5. `max_swap: i128` (USDC per swap)
    /// 6. `max_invested: i128` (USDC value of held RWA)
    /// 7. `max_spread_bps: u32`
    /// 8. `slippage_bps: u32`
    ///
    /// Panics on bad input by design: a failed deploy persists nothing. `paused` starts
    /// false (the empty allowlist already fails deposits closed), `auto_invest` false,
    /// `frozen_until` 0 and `keeper_freeze_spent` false. The
    /// venue is stored in `Config` and never changed (§3.1 step 4).
    fn __constructor(e: Env, asset: Address, init_args: Vec<Val>) {
        let rwa_asset: Address = init_args.get(0).expect("init_args[0]: rwa_asset").into_val(&e);
        let admin: Address = init_args.get(1).expect("init_args[1]: admin").into_val(&e);
        let keeper: Address = init_args.get(2).expect("init_args[2]: keeper").into_val(&e);
        let oracles: Vec<OracleSource> = init_args.get(3).expect("init_args[3]: oracles").into_val(&e);
        let venue: Venue = init_args.get(4).expect("init_args[4]: venue").into_val(&e);
        let max_swap: i128 = init_args.get(5).expect("init_args[5]: max_swap").into_val(&e);
        let max_invested: i128 = init_args.get(6).expect("init_args[6]: max_invested").into_val(&e);
        let max_spread_bps: u32 = init_args.get(7).expect("init_args[7]: max_spread_bps").into_val(&e);
        let slippage_bps: u32 = init_args.get(8).expect("init_args[8]: slippage_bps").into_val(&e);

        assert!(asset != rwa_asset, "asset must differ from rwa_asset");
        assert!(max_swap > 0, "max_swap must be positive");
        assert!(max_invested > 0, "max_invested must be positive");
        assert!((MIN_SPREAD_BPS..=MAX_SPREAD_BPS).contains(&max_spread_bps), "max_spread_bps out of range");
        assert!(slippage_bps <= MAX_SLIPPAGE_BPS, "slippage_bps above hard cap");

        let asset_decimals = TokenClient::new(&e, &asset).decimals();
        let rwa_decimals = TokenClient::new(&e, &rwa_asset).decimals();
        assert!(asset_decimals <= constants::MAX_DECIMALS, "asset decimals too large");
        assert!(rwa_decimals <= constants::MAX_DECIMALS, "rwa decimals too large");

        let config = Config { asset, rwa_asset, asset_decimals, rwa_decimals, venue };
        oracles::validate_oracles(&e, &oracles).expect("oracles: count, ttl, decimals, reachability");
        venue::validate_venue(&e, &config, &config.venue).expect("venue: token order");

        storage::set_config(&e, &config);
        storage::set_params(
            &e,
            &Params {
                admin,
                pending_admin: None,
                keeper,
                oracles,
                max_swap,
                max_invested,
                max_spread_bps,
                slippage_bps,
                paused: false,
                auto_invest: false,
                frozen_until: 0,
                keeper_freeze_spent: false,
                otc_max_deviation_bps: 0,
            },
        );
        storage::set_total_shares(&e, 0);
    }

    fn asset(e: Env) -> Result<Address, StrategyError> {
        extend_instance_ttl(&e);
        Ok(storage::get_config(&e)?.asset)
    }

    /// Accepts the whole `amount` into idle USDC and mints at par against pre-deposit NAV.
    /// Then, when `auto_invest` is on and pricing is `Live`, buys one best-effort tranche of at
    /// most `amount` and `max_swap` (§3.2), never the idle buffer. Never fails over the tranche.
    /// Fails closed outside `Live` while RWA is held: a stale anchor would misprice the shares.
    fn deposit(e: Env, amount: i128, from: Address) -> Result<i128, StrategyError> {
        extend_instance_ttl(&e);
        check_positive_amount(amount)?;
        from.require_auth();
        if !is_authorized_depositor(&e, &from) {
            return Err(StrategyError::NotAuthorized);
        }
        let config = storage::get_config(&e)?;
        let params = storage::get_params(&e)?;
        let held = check_rwa_balance_anomaly(&e, &config);
        if params.paused {
            return Err(StrategyError::NotAuthorized);
        }
        let nav = pricing::nav_guarded(&e, &config, &params)?;
        if held > 0 && nav.state.mode != pricing::PricingMode::Live {
            return Err(StrategyError::ExternalError);
        }

        let this = e.current_contract_address();
        TokenClient::new(&e, &config.asset).transfer(&from, &this, &amount);
        let total_shares = storage::get_total_shares(&e);
        let minted = shares::assets_to_shares_down(amount, total_shares, nav.nav)?;
        if minted <= 0 {
            return Err(StrategyError::InvalidSharesMinted);
        }
        let new_vault_shares = storage::get_vault_shares(&e, &from)
            .checked_add(minted)
            .ok_or(StrategyError::UnderflowOverflow)?;
        let new_total = total_shares.checked_add(minted).ok_or(StrategyError::UnderflowOverflow)?;
        storage::set_vault_shares(&e, &from, new_vault_shares);
        storage::set_total_shares(&e, new_total);
        event::emit_deposit(&e, name(&e), amount, from);

        // Sized from this deposit alone, never the idle buffer.
        flows::auto_invest_tranche(&e, &config, &params, &nav.state, held, amount)?;

        // Trailing balance from the pre-swap NAV plus the amount: a tranche is valued at the
        // rate the gate vetted, so a deposit never trips its own price impact.
        let nav_after = nav.nav.checked_add(amount).ok_or(StrategyError::UnderflowOverflow)?;
        shares::shares_to_assets_down(new_vault_shares, new_total, nav_after)
    }

    /// Telemetry no-op. Keeper dual check. Emits `HarvestEvent { amount: 0, price_per_share }`
    /// in `Live` mode, `SpreadAlertEvent` otherwise (plus the `PricingModeEvent` the guarded
    /// pricing emits). Moves no funds.
    fn harvest(e: Env, from: Address, _data: Option<Bytes>) -> Result<(), StrategyError> {
        extend_instance_ttl(&e);
        let params = storage::get_params(&e)?;
        params.keeper.require_auth();
        if from != params.keeper {
            return Err(StrategyError::NotAuthorized);
        }
        let config = storage::get_config(&e)?;
        let _ = check_rwa_balance_anomaly(&e, &config);
        let nav = pricing::nav_guarded(&e, &config, &params)?;
        if nav.state.mode == pricing::PricingMode::Live {
            let pps = shares::price_per_share(nav.nav, storage::get_total_shares(&e))?;
            event::emit_harvest(&e, name(&e), 0, params.keeper, pps);
        } else {
            SpreadAlertEvent { strategy_name: name(&e), timestamp: e.ledger().timestamp() }.publish(&e);
        }
        Ok(())
    }

    /// Withdraws `amount` USDC to `to`: idle first, then sells RWA only for the shortfall,
    /// which must be at most `max_swap` (§3.3). Any pricing mode proceeds; the sale's
    /// `min_out` is the shortfall itself, so a short fill reverts instead of short-paying.
    /// Never allowlist-gated, never pausable.
    fn withdraw(e: Env, amount: i128, from: Address, to: Address) -> Result<i128, StrategyError> {
        extend_instance_ttl(&e);
        check_positive_amount(amount)?;
        from.require_auth();
        let config = storage::get_config(&e)?;
        let params = storage::get_params(&e)?;
        let held = check_rwa_balance_anomaly(&e, &config);
        let nav = pricing::nav_guarded(&e, &config, &params)?;
        let total_shares = storage::get_total_shares(&e);
        let vault_shares = storage::get_vault_shares(&e, &from);

        let vault_underlying = shares::shares_to_assets_down(vault_shares, total_shares, nav.nav)?;
        if amount > vault_underlying {
            return Err(StrategyError::InsufficientBalance);
        }
        let mut shares_to_burn = shares::assets_to_shares_up(amount, total_shares, nav.nav)?;
        if shares_to_burn > vault_shares {
            shares_to_burn = vault_shares;
        }

        let from_idle = nav.idle.min(amount);
        let shortfall = amount.checked_sub(from_idle).ok_or(StrategyError::UnderflowOverflow)?;
        let mut rwa_remaining = held;
        if shortfall > 0 {
            if shortfall > params.max_swap || nav.state.rate <= 0 {
                return Err(StrategyError::InsufficientBalance);
            }
            let mut rwa_in = flows::max_rwa_in(&e, &config, &params, nav.state.rate, shortfall)?;
            if rwa_in > held {
                rwa_in = held;
            }
            if rwa_in <= 0 {
                return Err(StrategyError::InsufficientBalance);
            }
            venue::sell(&e, &config, &config.venue, rwa_in, shortfall)?;
            rwa_remaining = held.checked_sub(rwa_in).ok_or(StrategyError::UnderflowOverflow)?;
        }

        let this = e.current_contract_address();
        TokenClient::new(&e, &config.asset).transfer(&this, &to, &amount);

        let remaining_vault_shares = vault_shares.checked_sub(shares_to_burn).ok_or(StrategyError::UnderflowOverflow)?;
        let remaining_total = total_shares.checked_sub(shares_to_burn).ok_or(StrategyError::UnderflowOverflow)?;
        storage::set_vault_shares(&e, &from, remaining_vault_shares);
        storage::set_total_shares(&e, remaining_total);
        storage::set_expected_rwa(&e, rwa_remaining);
        event::emit_withdraw(&e, name(&e), amount, from);

        // Trailing balance from the pre-trade NAV minus the amount shipped (the gate vetted
        // that NAV; re-pricing would compare against a pool this sale just moved).
        let nav_after = nav.nav.checked_sub(amount).ok_or(StrategyError::UnderflowOverflow)?.max(0);
        if remaining_vault_shares == 0 {
            Ok(0)
        } else {
            shares::shares_to_assets_down(remaining_vault_shares, remaining_total, nav_after)
        }
    }

    /// USDC value of `from`'s shares. Total: no storage writes, no TTL bumps, and no revert
    /// on any oracle or venue failure (§3.6).
    fn balance(e: Env, from: Address) -> Result<i128, StrategyError> {
        let vault_shares = storage::peek_vault_shares(&e, &from);
        if vault_shares == 0 {
            return Ok(0);
        }
        let config = storage::get_config(&e)?;
        let params = storage::get_params(&e)?;
        let nav = pricing::nav_readonly(&e, &config, &params)?;
        shares::shares_to_assets_down(vault_shares, storage::get_total_shares(&e), nav.nav)
    }
}

#[contractimpl]
impl StableBondWrapperStrategy {
    /// The immutable configuration, venue included.
    pub fn get_config(e: Env) -> Result<Config, StrategyError> {
        extend_instance_ttl(&e);
        storage::get_config(&e)
    }

    /// Pauses / unpauses `deposit`, `invest` and `invest_otc`. Admin or keeper. Never
    /// affects `withdraw` or `divest`.
    pub fn pause_deposits(e: Env, caller: Address, paused: bool) -> Result<(), StrategyError> {
        extend_instance_ttl(&e);
        caller.require_auth();
        let mut params = storage::get_params(&e)?;
        if caller != params.admin && caller != params.keeper {
            return Err(StrategyError::NotAuthorized);
        }
        params.paused = paused;
        storage::set_params(&e, &params);
        PauseEvent { strategy_name: name(&e), paused }.publish(&e);
        Ok(())
    }

    /// Adds `depositor` to the address allowlist. Admin only; idempotent.
    pub fn add_depositor(e: Env, depositor: Address) -> Result<(), StrategyError> {
        extend_instance_ttl(&e);
        require_admin(&storage::get_params(&e)?);
        storage::set_depositor(&e, &depositor, true);
        SetDepositorEvent { strategy_name: name(&e), depositor, allowed: true }.publish(&e);
        Ok(())
    }

    /// Removes `depositor` from the address allowlist. Admin only. Never affects withdrawals.
    pub fn remove_depositor(e: Env, depositor: Address) -> Result<(), StrategyError> {
        extend_instance_ttl(&e);
        require_admin(&storage::get_params(&e)?);
        storage::set_depositor(&e, &depositor, false);
        SetDepositorEvent { strategy_name: name(&e), depositor, allowed: false }.publish(&e);
        Ok(())
    }

    /// True when `depositor` is on the exact-address deposit allowlist.
    pub fn is_depositor(e: Env, depositor: Address) -> Result<bool, StrategyError> {
        extend_instance_ttl(&e);
        Ok(storage::is_depositor(&e, &depositor))
    }

    /// Allowlists every contract running the wasm `hash`. Admin only; idempotent.
    pub fn add_allowed_hash(e: Env, hash: BytesN<32>) -> Result<(), StrategyError> {
        extend_instance_ttl(&e);
        require_admin(&storage::get_params(&e)?);
        storage::set_allowed_hash(&e, &hash, true);
        SetAllowedHashEvent { strategy_name: name(&e), hash, allowed: true }.publish(&e);
        Ok(())
    }

    /// Removes a wasm `hash` from the allowlist. Admin only. Never affects withdrawals.
    pub fn remove_allowed_hash(e: Env, hash: BytesN<32>) -> Result<(), StrategyError> {
        extend_instance_ttl(&e);
        require_admin(&storage::get_params(&e)?);
        storage::set_allowed_hash(&e, &hash, false);
        SetAllowedHashEvent { strategy_name: name(&e), hash, allowed: false }.publish(&e);
        Ok(())
    }

    /// True when the wasm code `hash` is on the deposit allowlist.
    pub fn is_allowed_hash(e: Env, hash: BytesN<32>) -> Result<bool, StrategyError> {
        extend_instance_ttl(&e);
        Ok(storage::is_allowed_hash(&e, &hash))
    }

    /// True when `counterparty` may trade OTC with the strategy.
    pub fn is_otc_counterparty(e: Env, counterparty: Address) -> Result<bool, StrategyError> {
        extend_instance_ttl(&e);
        Ok(storage::is_otc_counterparty(&e, &counterparty))
    }

    /// The mutable parameter blob: roles, oracle set, knobs and the two flags.
    pub fn get_params(e: Env) -> Result<Params, StrategyError> {
        extend_instance_ttl(&e);
        storage::get_params(&e)
    }

    /// Turns the per-deposit tranche on or off. Admin only. Ships off (§1 item 8).
    pub fn set_auto_invest(e: Env, enabled: bool) -> Result<(), StrategyError> {
        extend_instance_ttl(&e);
        let mut params = storage::get_params(&e)?;
        require_admin(&params);
        params.auto_invest = enabled;
        storage::set_params(&e, &params);
        AutoInvestSetEvent { strategy_name: name(&e), enabled }.publish(&e);
        Ok(())
    }

    /// Buys RWA with `usdc_amount` idle USDC on the pool. Keeper only. Refused when paused,
    /// outside `Live` mode or while the oracle reads below the clamped rate (`pricing::can_buy`),
    /// above `max_swap`, above idle, or above the `max_invested` room.
    /// `min_out` is the quantity the oracle rate implies minus `slippage_bps`: a pool under
    /// NAV fills above it. Returns the RWA bought.
    pub fn invest(e: Env, usdc_amount: i128) -> Result<i128, StrategyError> {
        extend_instance_ttl(&e);
        let params = storage::get_params(&e)?;
        params.keeper.require_auth();
        check_positive_amount(usdc_amount)?;
        if params.paused {
            return Err(StrategyError::NotAuthorized);
        }
        if usdc_amount > params.max_swap {
            return Err(StrategyError::InvalidArgument);
        }
        let config = storage::get_config(&e)?;
        let held = check_rwa_balance_anomaly(&e, &config);
        let nav = pricing::nav_guarded(&e, &config, &params)?;
        if !pricing::can_buy(&nav.state) {
            return Err(StrategyError::ExternalError);
        }
        if usdc_amount > nav.idle {
            return Err(StrategyError::InsufficientBalance);
        }
        if usdc_amount > flows::invested_room(&e, &config, &params, nav.state.rate, held)? {
            return Err(StrategyError::InvalidArgument);
        }
        flows::buy_rwa(&e, &config, &params, nav.state.rate, held, usdc_amount)
    }

    /// Sells RWA on the pool to raise `usdc_target` idle USDC. Keeper only. Any mode with a
    /// rate; the sale's `min_out` is the target and it gives up at most the oracle-implied
    /// quantity plus `slippage_bps`, so a pool further under NAV than that reverts. Refused
    /// above `max_swap`. Returns the USDC received.
    pub fn divest(e: Env, usdc_target: i128) -> Result<i128, StrategyError> {
        extend_instance_ttl(&e);
        let params = storage::get_params(&e)?;
        params.keeper.require_auth();
        check_positive_amount(usdc_target)?;
        if usdc_target > params.max_swap {
            return Err(StrategyError::InvalidArgument);
        }
        let config = storage::get_config(&e)?;
        let held = check_rwa_balance_anomaly(&e, &config);
        let nav = pricing::nav_guarded(&e, &config, &params)?;
        if nav.state.rate <= 0 {
            return Err(StrategyError::ExternalError);
        }
        let mut rwa_in = flows::max_rwa_in(&e, &config, &params, nav.state.rate, usdc_target)?;
        if rwa_in > held {
            rwa_in = held;
        }
        if rwa_in <= 0 {
            return Err(StrategyError::InsufficientBalance);
        }
        let out = venue::sell(&e, &config, &config.venue, rwa_in, usdc_target)?;
        storage::set_expected_rwa(&e, held.checked_sub(rwa_in).ok_or(StrategyError::UnderflowOverflow)?);
        DivestEvent { strategy_name: name(&e), rwa_in, usdc_out: out }.publish(&e);
        Ok(out)
    }

    /// OTC buy from an allowlisted counterparty at `price`, the price agreed with it, in the
    /// unit of `get_pricing_state().rate` (§2.3). Refused with `InvalidArgument` unless `price`
    /// is within `otc_max_deviation_bps` of the gated rate (§3.8). Keeper only. Returns the
    /// RWA pulled.
    pub fn invest_otc(e: Env, counterparty: Address, usdc_amount: i128, price: i128) -> Result<i128, StrategyError> {
        extend_instance_ttl(&e);
        let params = storage::get_params(&e)?;
        params.keeper.require_auth();
        check_positive_amount(usdc_amount)?;
        check_positive_amount(price)?;
        let config = storage::get_config(&e)?;
        let _ = check_rwa_balance_anomaly(&e, &config);
        otc::invest(&e, &config, &params, &counterparty, usdc_amount, price)
    }

    /// OTC sell to an allowlisted counterparty at `price`, bounded exactly like `invest_otc`
    /// (§3.8). Keeper only. Works in every mode with a rate, `Frozen` included; this is how
    /// idle is refilled without selling into the pool. Returns the RWA paid.
    pub fn divest_otc(e: Env, counterparty: Address, usdc_amount: i128, price: i128) -> Result<i128, StrategyError> {
        extend_instance_ttl(&e);
        let params = storage::get_params(&e)?;
        params.keeper.require_auth();
        check_positive_amount(usdc_amount)?;
        check_positive_amount(price)?;
        let config = storage::get_config(&e)?;
        let _ = check_rwa_balance_anomaly(&e, &config);
        otc::divest(&e, &config, &params, &counterparty, usdc_amount, price)
    }

    /// The pricing result for the bot: rate, raw oracle rate, mode, live oracle count.
    pub fn get_pricing_state(e: Env) -> Result<PricingState, StrategyError> {
        extend_instance_ttl(&e);
        let config = storage::get_config(&e)?;
        let params = storage::get_params(&e)?;
        pricing::price_rwa(&e, &config, &params)
    }

    /// Proposes a new oracle set, executable `TIMELOCK_SECONDS` later (§3.7). Same
    /// validation as the constructor. Admin only. Returns `executable_at`.
    pub fn propose_oracles(e: Env, oracles: Vec<OracleSource>) -> Result<u64, StrategyError> {
        extend_instance_ttl(&e);
        require_admin(&storage::get_params(&e)?);
        timelock::propose_oracles(&e, oracles)
    }

    /// Applies the pending oracle set once its time has come and at least one proposed source
    /// reads live (§3.7, liveness only). Admin only. Returns the new source count.
    pub fn execute_oracles(e: Env) -> Result<u32, StrategyError> {
        extend_instance_ttl(&e);
        let params = storage::get_params(&e)?;
        require_admin(&params);
        let config = storage::get_config(&e)?;
        timelock::execute_oracles(&e, &config, &params)
    }

    /// Drops the pending oracle proposal. Admin only.
    pub fn cancel_oracles(e: Env) -> Result<(), StrategyError> {
        extend_instance_ttl(&e);
        require_admin(&storage::get_params(&e)?);
        timelock::cancel_oracles(&e)
    }

    /// Proposes new values for the four price knobs — `max_spread_bps`, `slippage_bps`,
    /// `max_swap` and `otc_max_deviation_bps` — executable `TIMELOCK_SECONDS` later (§6, §9 Q11).
    /// This is the only way any of the four goes *up*. Admin only. Returns `executable_at`.
    pub fn propose_params(
        e: Env,
        max_spread_bps: u32,
        slippage_bps: u32,
        max_swap: i128,
        otc_max_deviation_bps: u32,
    ) -> Result<u64, StrategyError> {
        extend_instance_ttl(&e);
        require_admin(&storage::get_params(&e)?);
        timelock::propose_params(&e, max_spread_bps, slippage_bps, max_swap, otc_max_deviation_bps)
    }

    /// Applies the pending knobs after the wait (§6). Admin only.
    pub fn execute_params(e: Env) -> Result<(), StrategyError> {
        extend_instance_ttl(&e);
        let params = storage::get_params(&e)?;
        require_admin(&params);
        timelock::execute_params(&e, &params)
    }

    /// Drops the pending knob proposal. Admin only.
    pub fn cancel_params(e: Env) -> Result<(), StrategyError> {
        extend_instance_ttl(&e);
        require_admin(&storage::get_params(&e)?);
        timelock::cancel_params(&e)
    }

    /// Narrows the OTC price band immediately (§3.8). Widening it needs `propose_params`, so
    /// any room for a fill further from the rate is always announced 72 h ahead. Admin only.
    pub fn set_otc_max_deviation_bps(e: Env, bps: u32) -> Result<(), StrategyError> {
        extend_instance_ttl(&e);
        let mut params = storage::get_params(&e)?;
        require_admin(&params);
        if bps > MAX_OTC_DEVIATION_BPS {
            return Err(StrategyError::InvalidArgument);
        }
        if bps > params.otc_max_deviation_bps {
            return Err(StrategyError::NotAuthorized);
        }
        let old_band = params.otc_max_deviation_bps;
        params.otc_max_deviation_bps = bps;
        storage::set_params(&e, &params);
        emit_params_update(&e, "otc_max_deviation_bps", old_band as i128, bps as i128);
        Ok(())
    }

    /// Announces an OTC counterparty, allowlistable `TIMELOCK_SECONDS` later (§3.8). Admin
    /// only. Returns `executable_at`.
    pub fn propose_otc_counterparty(e: Env, counterparty: Address) -> Result<u64, StrategyError> {
        extend_instance_ttl(&e);
        require_admin(&storage::get_params(&e)?);
        timelock::propose_counterparty(&e, &counterparty)
    }

    /// Allowlists a proposed counterparty once its time has come. Admin only.
    pub fn execute_otc_counterparty(e: Env, counterparty: Address) -> Result<(), StrategyError> {
        extend_instance_ttl(&e);
        require_admin(&storage::get_params(&e)?);
        timelock::execute_counterparty(&e, &counterparty)
    }

    /// Removes a counterparty, and any pending proposal for it, immediately. Admin only.
    pub fn remove_otc_counterparty(e: Env, counterparty: Address) -> Result<(), StrategyError> {
        extend_instance_ttl(&e);
        require_admin(&storage::get_params(&e)?);
        timelock::remove_counterparty(&e, &counterparty);
        Ok(())
    }

    /// The pending oracle proposal, if any.
    pub fn get_pending_oracles(e: Env) -> Option<PendingOracles> {
        extend_instance_ttl(&e);
        storage::get_pending_oracles(&e)
    }

    /// The pending knob proposal, if any.
    pub fn get_pending_params(e: Env) -> Option<PendingParams> {
        extend_instance_ttl(&e);
        storage::get_pending_params(&e)
    }

    /// `executable_at` of a proposed counterparty, if any.
    pub fn get_pending_otc_counterparty(e: Env, counterparty: Address) -> Option<u64> {
        extend_instance_ttl(&e);
        storage::get_pending_otc_counterparty(&e, &counterparty)
    }

    /// Starts a two-step admin transfer. Admin only. Losing the admin on a non-upgradable
    /// contract loses every knob, so there is no one-shot rotation.
    pub fn transfer_admin(e: Env, new_admin: Address) -> Result<(), StrategyError> {
        extend_instance_ttl(&e);
        let mut params = storage::get_params(&e)?;
        require_admin(&params);
        params.pending_admin = Some(new_admin.clone());
        storage::set_params(&e, &params);
        AdminTransferStartedEvent { strategy_name: name(&e), admin: params.admin.clone(), pending_admin: new_admin }.publish(&e);
        Ok(())
    }

    /// Completes the transfer. The pending admin must sign.
    pub fn accept_admin(e: Env) -> Result<(), StrategyError> {
        extend_instance_ttl(&e);
        let mut params = storage::get_params(&e)?;
        let pending = params.pending_admin.clone().ok_or(StrategyError::NotAuthorized)?;
        pending.require_auth();
        let old_admin = params.admin.clone();
        params.admin = pending.clone();
        params.pending_admin = None;
        storage::set_params(&e, &params);
        AdminTransferredEvent { strategy_name: name(&e), old_admin, new_admin: pending }.publish(&e);
        Ok(())
    }

    /// Replaces the keeper. Admin only, immediate: the bot key must be replaceable in one
    /// transaction when compromised, and the keeper cannot move funds out (§11). Re-arms the
    /// keeper's one `freeze_price`, so the new key starts with it.
    pub fn set_keeper(e: Env, new_keeper: Address) -> Result<(), StrategyError> {
        extend_instance_ttl(&e);
        let mut params = storage::get_params(&e)?;
        require_admin(&params);
        let old_keeper = params.keeper.clone();
        params.keeper = new_keeper.clone();
        params.keeper_freeze_spent = false;
        storage::set_params(&e, &params);
        SetKeeperEvent { strategy_name: name(&e), old_keeper, new_keeper }.publish(&e);
        Ok(())
    }

    /// Sets `max_swap` and `max_invested`, both positive. **`max_swap` may only be lowered
    /// here** (`NotAuthorized` on a raise, §9 Q11): it bounds the strategy's own price impact on
    /// one pool, so a raise goes through `propose_params` and waits 72 h in public.
    /// `max_invested` moves freely either way — it caps exposure, not price. Admin only.
    pub fn set_caps(e: Env, max_swap: i128, max_invested: i128) -> Result<(), StrategyError> {
        extend_instance_ttl(&e);
        let mut params = storage::get_params(&e)?;
        require_admin(&params);
        if max_swap <= 0 || max_invested <= 0 {
            return Err(StrategyError::InvalidArgument);
        }
        if max_swap > params.max_swap {
            return Err(StrategyError::NotAuthorized);
        }
        let (old_swap, old_inv) = (params.max_swap, params.max_invested);
        params.max_swap = max_swap;
        params.max_invested = max_invested;
        storage::set_params(&e, &params);
        emit_params_update(&e, "max_swap", old_swap, max_swap);
        emit_params_update(&e, "max_invested", old_inv, max_invested);
        Ok(())
    }

    /// Sets the spread and slippage with the constructor's bounds. **Both may only be
    /// lowered here** (`NotAuthorized` on a raise, §9 Q11): composed they price the strategy's
    /// own trades, so a raise goes through `propose_params` and waits 72 h in public. Admin only.
    pub fn set_pricing_params(e: Env, max_spread_bps: u32, slippage_bps: u32) -> Result<(), StrategyError> {
        extend_instance_ttl(&e);
        let mut params = storage::get_params(&e)?;
        require_admin(&params);
        if !(MIN_SPREAD_BPS..=MAX_SPREAD_BPS).contains(&max_spread_bps) || slippage_bps > MAX_SLIPPAGE_BPS {
            return Err(StrategyError::InvalidArgument);
        }
        if max_spread_bps > params.max_spread_bps || slippage_bps > params.slippage_bps {
            return Err(StrategyError::NotAuthorized);
        }
        let (old_spread, old_slip) = (params.max_spread_bps, params.slippage_bps);
        params.max_spread_bps = max_spread_bps;
        params.slippage_bps = slippage_bps;
        storage::set_params(&e, &params);
        emit_params_update(&e, "max_spread_bps", old_spread as i128, max_spread_bps as i128);
        emit_params_update(&e, "slippage_bps", old_slip as i128, slippage_bps as i128);
        Ok(())
    }

    /// Freezes (`true`) or unfreezes (`false`) the rate at the anchor (§3.5, `PricingMode::Frozen`).
    /// The admin freezes until it unfreezes; the keeper freezes for `KEEPER_FREEZE_SECONDS`
    /// from now (never shortening a longer freeze), **once**, and cannot unfreeze. The keeper's
    /// freeze is spent until the admin unfreezes or calls `set_keeper`, so a compromised keeper
    /// holds the rate for at most `KEEPER_FREEZE_SECONDS` without the multisig acting. Immediate
    /// both ways: freezing can only hold the last agreed price, never set one, and after a
    /// freeze ends the rate still rejoins the oracle at most `max_spread_bps` per
    /// `ANCHOR_WINDOW_SECONDS`. While frozen, deposits against held RWA, `invest` and
    /// `invest_otc` refuse; `withdraw`, `divest` and `divest_otc` price at the anchor.
    pub fn freeze_price(e: Env, caller: Address, frozen: bool) -> Result<(), StrategyError> {
        extend_instance_ttl(&e);
        caller.require_auth();
        let mut params = storage::get_params(&e)?;
        if caller == params.admin {
            params.frozen_until = if frozen { u64::MAX } else { 0 };
            if !frozen {
                params.keeper_freeze_spent = false;
            }
        } else if frozen && caller == params.keeper && !params.keeper_freeze_spent {
            let until = e.ledger().timestamp().saturating_add(KEEPER_FREEZE_SECONDS);
            params.frozen_until = params.frozen_until.max(until);
            params.keeper_freeze_spent = true;
        } else {
            return Err(StrategyError::NotAuthorized);
        }
        storage::set_params(&e, &params);
        PriceFrozenEvent { strategy_name: name(&e), caller, frozen, frozen_until: params.frozen_until }.publish(&e);
        Ok(())
    }

    /// Lowers the anchor toward the oracle rate after a real impairment the admin has confirmed
    /// (Halborn FIND-002), instead of letting `Live` pricing book it one `max_spread_bps` step
    /// per `ANCHOR_WINDOW_SECONDS`. Admin only. `expected_rate` is the rate (§2.3 scale) the
    /// multisig checked; every configured oracle must be live and within `max_spread_bps` of it,
    /// and their minimum must sit under the anchor, so this can only lower NAV, and only toward
    /// what the oracles say. Each call moves the anchor at most `MAX_REANCHOR_STEP_BPS` (5%), and
    /// one call is allowed per `ANCHOR_WINDOW_SECONDS`; a larger drop takes several calls.
    /// Refused while frozen (`ExternalError`; the admin unfreezes first) and with any source
    /// down (`ExternalError`); refused upward, within a window of the last reanchor, with no
    /// anchor, or off the admin's figure (`InvalidArgument`). Restarts the anchor window.
    /// Returns the new anchor.
    pub fn reanchor_to_oracle(e: Env, expected_rate: i128) -> Result<i128, StrategyError> {
        extend_instance_ttl(&e);
        let params = storage::get_params(&e)?;
        require_admin(&params);
        let config = storage::get_config(&e)?;
        let (old_rate, new_rate) = pricing::reanchor(&e, &config, &params, expected_rate)?;
        ReanchorEvent { strategy_name: name(&e), old_rate, new_rate, expected_rate }.publish(&e);
        Ok(new_rate)
    }
}

#[cfg(test)]
mod testutils;
#[cfg(test)]
mod test_totality;
#[cfg(test)]
mod test_constructor;
#[cfg(test)]
mod test_oracles;
#[cfg(test)]
mod test_pricing;
#[cfg(test)]
mod test_venue_aqua;
#[cfg(test)]
mod test_venue_sushi;
#[cfg(test)]
mod test_deposit;
#[cfg(test)]
mod test_withdraw;
#[cfg(test)]
mod test_keeper;
#[cfg(test)]
mod test_timelock;
#[cfg(test)]
mod test_otc;
#[cfg(test)]
mod test_admin;
#[cfg(test)]
mod test_reanchor;
#[cfg(test)]
mod integration_test;
