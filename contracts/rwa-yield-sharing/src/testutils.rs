#![cfg(test)]
//! Test doubles for the contract surface and the shared fixture. Every mock has `set_panic(true)`
//! so each `try_` path in the strategy is exercised; the "absent contract" case is proven
//! once in `test_totality.rs`. Prices are USDC stroops per whole RWA (7 dp): `PRICE_ONE`
//! is 1.00. The Aqua and Sushi mocks fill at that price with an optional short-fill.

extern crate std;

use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{Address as _, Ledger as _},
    token::{StellarAssetClient, TokenClient},
    Address, Env, IntoVal, Vec, U256,
};

use crate::abi::{OracleHints, RedStonePriceData, SwapResult};
use crate::storage::{Config, OracleKind, OracleSource, Venue};
use crate::{RwaYieldSharingStrategy, RwaYieldSharingStrategyClient, StrategyError};

pub const ONE: i128 = 10_000_000; // 7 dp
pub const ONE_18: i128 = 1_000_000_000_000_000_000;
pub const PRICE_ONE: i128 = ONE; // 1.00 USDC per whole RWA
pub const SEP40_DECIMALS: u32 = 14;
pub const REDSTONE_DECIMALS: u32 = 8;
pub const START_TS: u64 = 1_000_000;

// ===========================================================================
// MockToken: SEP-41 shaped token with configurable decimals (SACs are always 7 dp, and
// deJTRSY is 18). Implements exactly what the strategy and the OTC path call.
// ===========================================================================

#[contracttype]
enum TKey {
    Decimals,
    Balance(Address),
    Allowance(Address, Address),
}

#[contract]
pub struct MockToken;

#[contractimpl]
impl MockToken {
    pub fn __constructor(e: Env, decimals: u32) {
        e.storage().instance().set(&TKey::Decimals, &decimals);
    }

    pub fn decimals(e: Env) -> u32 {
        e.storage().instance().get(&TKey::Decimals).unwrap()
    }

    pub fn balance(e: Env, id: Address) -> i128 {
        e.storage().persistent().get(&TKey::Balance(id)).unwrap_or(0)
    }

    pub fn mint(e: Env, to: Address, amount: i128) {
        let b = Self::balance(e.clone(), to.clone());
        e.storage().persistent().set(&TKey::Balance(to), &(b + amount));
    }

    pub fn transfer(e: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        Self::move_(&e, &from, &to, amount);
    }

    pub fn approve(e: Env, from: Address, spender: Address, amount: i128, _expiration_ledger: u32) {
        from.require_auth();
        e.storage().persistent().set(&TKey::Allowance(from, spender), &amount);
    }

    pub fn allowance(e: Env, from: Address, spender: Address) -> i128 {
        e.storage().persistent().get(&TKey::Allowance(from, spender)).unwrap_or(0)
    }

    pub fn transfer_from(e: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();
        let a = Self::allowance(e.clone(), from.clone(), spender.clone());
        assert!(a >= amount, "insufficient allowance");
        e.storage().persistent().set(&TKey::Allowance(from.clone(), spender), &(a - amount));
        Self::move_(&e, &from, &to, amount);
    }

    fn move_(e: &Env, from: &Address, to: &Address, amount: i128) {
        let fb = Self::balance(e.clone(), from.clone());
        assert!(fb >= amount, "insufficient balance");
        e.storage().persistent().set(&TKey::Balance(from.clone()), &(fb - amount));
        let tb = Self::balance(e.clone(), to.clone());
        e.storage().persistent().set(&TKey::Balance(to.clone()), &(tb + amount));
    }
}

// ===========================================================================
// MockSep40: Reflector-shaped feed. `set_missing` makes lastprice return None.
// ===========================================================================

#[contracttype]
enum OKey {
    Dec,
    Price,
    Ts,
    Missing,
    Panic,
}

fn panicking(e: &Env, key: &OKey) {
    if e.storage().instance().get::<OKey, bool>(key).unwrap_or(false) {
        panic!("mock configured to panic");
    }
}

#[contract]
pub struct MockSep40;

#[contractimpl]
impl MockSep40 {
    pub fn __constructor(e: Env, decimals: u32, price: i128, timestamp: u64) {
        e.storage().instance().set(&OKey::Dec, &decimals);
        e.storage().instance().set(&OKey::Price, &price);
        e.storage().instance().set(&OKey::Ts, &timestamp);
    }
    pub fn set_price(e: Env, price: i128, timestamp: u64) {
        e.storage().instance().set(&OKey::Price, &price);
        e.storage().instance().set(&OKey::Ts, &timestamp);
    }
    pub fn set_missing(e: Env, missing: bool) {
        e.storage().instance().set(&OKey::Missing, &missing);
    }
    pub fn set_panic(e: Env, on: bool) {
        e.storage().instance().set(&OKey::Panic, &on);
    }
    pub fn decimals(e: Env) -> u32 {
        panicking(&e, &OKey::Panic);
        e.storage().instance().get(&OKey::Dec).unwrap()
    }
    pub fn lastprice(e: Env, _asset: sep_40_oracle::Asset) -> Option<sep_40_oracle::PriceData> {
        panicking(&e, &OKey::Panic);
        if e.storage().instance().get::<OKey, bool>(&OKey::Missing).unwrap_or(false) {
            return None;
        }
        Some(sep_40_oracle::PriceData {
            price: e.storage().instance().get(&OKey::Price).unwrap(),
            timestamp: e.storage().instance().get(&OKey::Ts).unwrap(),
        })
    }
}

// ===========================================================================
// MockRedStone: per-asset adapter. decimals() -> u64, milliseconds, U256 price.
// ===========================================================================

#[contracttype]
enum RKey {
    Dec,
    Price,
    TsMs,
    Panic,
}

#[contract]
pub struct MockRedStone;

#[contractimpl]
impl MockRedStone {
    pub fn __constructor(e: Env, decimals: u64, price: u128, package_ts_ms: u64) {
        e.storage().instance().set(&RKey::Dec, &decimals);
        e.storage().instance().set(&RKey::Price, &price);
        e.storage().instance().set(&RKey::TsMs, &package_ts_ms);
    }
    pub fn set_price(e: Env, price: u128, package_ts_ms: u64) {
        e.storage().instance().set(&RKey::Price, &price);
        e.storage().instance().set(&RKey::TsMs, &package_ts_ms);
    }
    pub fn set_panic(e: Env, on: bool) {
        e.storage().instance().set(&RKey::Panic, &on);
    }
    pub fn decimals(e: Env) -> u64 {
        if e.storage().instance().get::<RKey, bool>(&RKey::Panic).unwrap_or(false) {
            panic!("mock configured to panic");
        }
        e.storage().instance().get(&RKey::Dec).unwrap()
    }
    pub fn read_price_data(e: Env) -> RedStonePriceData {
        if e.storage().instance().get::<RKey, bool>(&RKey::Panic).unwrap_or(false) {
            panic!("mock configured to panic");
        }
        let ts: u64 = e.storage().instance().get(&RKey::TsMs).unwrap();
        let price: u128 = e.storage().instance().get(&RKey::Price).unwrap();
        RedStonePriceData { package_timestamp: ts, price: U256::from_u128(&e, price), write_timestamp: ts }
    }
}

// ===========================================================================
// MockAquaPool: get_tokens / estimate_swap / swap at a linear price. `swap` pulls token_in
// from `user` with transfer(user -> pool), the depth-2 pull the strategy must grant.
// `fill_bps` shorts the executed payout relative to the quote (default 10_000).
// ===========================================================================

#[contracttype]
enum AKey {
    Tokens,
    Usdc,
    RwaDec,
    Price,
    FillBps,
    Panic,
}

fn convert(e: &Env, token_in_is_usdc: bool, in_amount: i128, price: i128, rwa_dec: u32) -> i128 {
    let unit = 10i128.pow(rwa_dec);
    let num = if token_in_is_usdc {
        soroban_sdk::I256::from_i128(e, in_amount).mul(&soroban_sdk::I256::from_i128(e, unit))
    } else {
        soroban_sdk::I256::from_i128(e, in_amount).mul(&soroban_sdk::I256::from_i128(e, price))
    };
    let den = if token_in_is_usdc { price } else { unit };
    num.div(&soroban_sdk::I256::from_i128(e, den)).to_i128().unwrap()
}

#[contract]
pub struct MockAquaPool;

#[contractimpl]
impl MockAquaPool {
    pub fn __constructor(e: Env, tokens: Vec<Address>, usdc: Address, rwa_decimals: u32, price: i128) {
        e.storage().instance().set(&AKey::Tokens, &tokens);
        e.storage().instance().set(&AKey::Usdc, &usdc);
        e.storage().instance().set(&AKey::RwaDec, &rwa_decimals);
        e.storage().instance().set(&AKey::Price, &price);
    }
    pub fn set_price(e: Env, price: i128) {
        e.storage().instance().set(&AKey::Price, &price);
    }
    pub fn set_fill_bps(e: Env, bps: i128) {
        e.storage().instance().set(&AKey::FillBps, &bps);
    }
    pub fn set_panic(e: Env, on: bool) {
        e.storage().instance().set(&AKey::Panic, &on);
    }
    fn guard(e: &Env) {
        if e.storage().instance().get::<AKey, bool>(&AKey::Panic).unwrap_or(false) {
            panic!("mock configured to panic");
        }
    }
    pub fn get_tokens(e: Env) -> Vec<Address> {
        Self::guard(&e);
        e.storage().instance().get(&AKey::Tokens).unwrap()
    }
    fn quote(e: &Env, in_idx: u32, in_amount: u128) -> u128 {
        let tokens: Vec<Address> = e.storage().instance().get(&AKey::Tokens).unwrap();
        let usdc: Address = e.storage().instance().get(&AKey::Usdc).unwrap();
        let price: i128 = e.storage().instance().get(&AKey::Price).unwrap();
        let dec: u32 = e.storage().instance().get(&AKey::RwaDec).unwrap();
        convert(e, tokens.get(in_idx).unwrap() == usdc, in_amount as i128, price, dec) as u128
    }
    pub fn estimate_swap(e: Env, in_idx: u32, _out_idx: u32, in_amount: u128) -> u128 {
        Self::guard(&e);
        Self::quote(&e, in_idx, in_amount)
    }
    pub fn swap(e: Env, user: Address, in_idx: u32, out_idx: u32, in_amount: u128, out_min: u128) -> u128 {
        Self::guard(&e);
        user.require_auth();
        let fill: i128 = e.storage().instance().get(&AKey::FillBps).unwrap_or(10_000);
        let out = (Self::quote(&e, in_idx, in_amount) as i128 * fill / 10_000) as u128;
        assert!(out >= out_min, "OutMinNotSatisfied");
        let tokens: Vec<Address> = e.storage().instance().get(&AKey::Tokens).unwrap();
        let this = e.current_contract_address();
        TokenClient::new(&e, &tokens.get(in_idx).unwrap()).transfer(&user, &this, &(in_amount as i128));
        TokenClient::new(&e, &tokens.get(out_idx).unwrap()).transfer(&this, &user, &(out as i128));
        out
    }
}

// ===========================================================================
// MockSushiPool: token0/token1/get_oracle_hints/quote_exact_input/swap. `swap` pulls
// token_in from `sender`. Received leg is the negative field, as on chain.
// ===========================================================================

#[contracttype]
enum SKey {
    Token0,
    Token1,
    Usdc,
    RwaDec,
    Price,
    FillBps,
    Panic,
}

#[contract]
pub struct MockSushiPool;

#[contractimpl]
impl MockSushiPool {
    pub fn __constructor(e: Env, token0: Address, token1: Address, usdc: Address, rwa_decimals: u32, price: i128) {
        e.storage().instance().set(&SKey::Token0, &token0);
        e.storage().instance().set(&SKey::Token1, &token1);
        e.storage().instance().set(&SKey::Usdc, &usdc);
        e.storage().instance().set(&SKey::RwaDec, &rwa_decimals);
        e.storage().instance().set(&SKey::Price, &price);
    }
    pub fn set_price(e: Env, price: i128) {
        e.storage().instance().set(&SKey::Price, &price);
    }
    pub fn set_fill_bps(e: Env, bps: i128) {
        e.storage().instance().set(&SKey::FillBps, &bps);
    }
    pub fn set_panic(e: Env, on: bool) {
        e.storage().instance().set(&SKey::Panic, &on);
    }
    fn guard(e: &Env) {
        if e.storage().instance().get::<SKey, bool>(&SKey::Panic).unwrap_or(false) {
            panic!("mock configured to panic");
        }
    }
    pub fn token0(e: Env) -> Address {
        Self::guard(&e);
        e.storage().instance().get(&SKey::Token0).unwrap()
    }
    pub fn token1(e: Env) -> Address {
        Self::guard(&e);
        e.storage().instance().get(&SKey::Token1).unwrap()
    }
    pub fn get_oracle_hints(e: Env) -> OracleHints {
        Self::guard(&e);
        OracleHints { checkpoint: 0, checkpoint_min: 0, slot: 0 }
    }
    fn quote(e: &Env, zero_for_one: bool, amount_in: i128) -> i128 {
        let t_in: Address = if zero_for_one {
            e.storage().instance().get(&SKey::Token0).unwrap()
        } else {
            e.storage().instance().get(&SKey::Token1).unwrap()
        };
        let usdc: Address = e.storage().instance().get(&SKey::Usdc).unwrap();
        let price: i128 = e.storage().instance().get(&SKey::Price).unwrap();
        let dec: u32 = e.storage().instance().get(&SKey::RwaDec).unwrap();
        convert(e, t_in == usdc, amount_in, price, dec)
    }
    fn result(e: &Env, zero_for_one: bool, amount_in: i128, out: i128) -> SwapResult {
        let (amount0, amount1) = if zero_for_one { (amount_in, -out) } else { (-out, amount_in) };
        SwapResult { amount0, amount1, liquidity: 0, sqrt_price_x96: U256::from_u128(e, 0), tick: 0 }
    }
    pub fn quote_exact_input(e: Env, zero_for_one: bool, amount_in: i128, _limit: U256) -> SwapResult {
        Self::guard(&e);
        let out = Self::quote(&e, zero_for_one, amount_in);
        Self::result(&e, zero_for_one, amount_in, out)
    }
    pub fn swap(
        e: Env,
        sender: Address,
        recipient: Address,
        zero_for_one: bool,
        amount_specified: i128,
        _limit: U256,
        _hints: OracleHints,
    ) -> SwapResult {
        Self::guard(&e);
        sender.require_auth();
        let fill: i128 = e.storage().instance().get(&SKey::FillBps).unwrap_or(10_000);
        let out = Self::quote(&e, zero_for_one, amount_specified) * fill / 10_000;
        let (t_in, t_out): (Address, Address) = if zero_for_one {
            (e.storage().instance().get(&SKey::Token0).unwrap(), e.storage().instance().get(&SKey::Token1).unwrap())
        } else {
            (e.storage().instance().get(&SKey::Token1).unwrap(), e.storage().instance().get(&SKey::Token0).unwrap())
        };
        let this = e.current_contract_address();
        TokenClient::new(&e, &t_in).transfer(&sender, &this, &amount_specified);
        TokenClient::new(&e, &t_out).transfer(&this, &recipient, &out);
        Self::result(&e, zero_for_one, amount_specified, out)
    }
}

// ===========================================================================
// MockVault: a contract depositor (only contracts carry a wasm hash).
// ===========================================================================

#[contract]
pub struct MockVault;

#[contractimpl]
impl MockVault {
    pub fn __constructor(_e: Env) {}
    pub fn go_deposit(e: Env, strategy: Address, amount: i128) -> i128 {
        RwaYieldSharingStrategyClient::new(&e, &strategy).deposit(&amount, &e.current_contract_address())
    }
}

// ===========================================================================
// Probes: run crate-internal venue code inside a real contract frame.
// ===========================================================================

#[contract]
pub struct VenueProbe;

#[contractimpl]
impl VenueProbe {
    pub fn sell(e: Env, config: Config, venue: Venue, rwa_in: i128, min_out: i128) -> Result<i128, StrategyError> {
        crate::venue::sell(&e, &config, &venue, rwa_in, min_out)
    }
    pub fn buy(e: Env, config: Config, venue: Venue, usdc_in: i128, min_out: i128) -> Result<i128, StrategyError> {
        crate::venue::buy(&e, &config, &venue, usdc_in, min_out)
    }
}

/// Grants BEFORE reading hints: the grant is scoped to the next invocation's subtree, the
/// hints read consumes it, and the swap's pull is unauthorized. Exists to prove the ordering
/// in `venue_sushi::swap` is load-bearing.
#[contract]
pub struct SushiMisorderedProbe;

#[contractimpl]
impl SushiMisorderedProbe {
    pub fn swap_grant_before_hints(e: Env, pool: Address, token_in: Address, zero_for_one: bool, amount: i128) -> i128 {
        let me = e.current_contract_address();
        crate::venue::grant_transfer(&e, &me, &token_in, &pool, amount);
        let client = crate::abi::SushiPoolClient::new(&e, &pool);
        let hints = client.get_oracle_hints();
        let bound = U256::from_u128(&e, 4_295_128_740);
        let r = client.swap(&me, &me, &zero_for_one, &amount, &bound, &hints);
        if zero_for_one { -r.amount1 } else { -r.amount0 }
    }
}

// ===========================================================================
// Fixture
// ===========================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VenueKind {
    Aqua,
    Sushi,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OracleSet {
    RedStoneOnly,
    Both,
}

#[derive(Clone, Copy)]
pub struct Cfg {
    pub rwa_decimals: u32,
    pub venue: VenueKind,
    pub oracles: OracleSet,
    pub max_swap: i128,
    pub max_invested: i128,
    pub max_spread_bps: u32,
    pub slippage_bps: u32,
    pub sep40_ttl: u64,
    pub redstone_ttl: u64,
}

/// USTRY-like: 7 dp RWA on Aqua with Reflector + RedStone.
pub fn cfg_aqua7() -> Cfg {
    Cfg {
        rwa_decimals: 7,
        venue: VenueKind::Aqua,
        oracles: OracleSet::Both,
        max_swap: 10_000 * ONE,
        max_invested: 1_000_000 * ONE,
        max_spread_bps: 100,
        slippage_bps: 100,
        sep40_ttl: 900,
        // MAX_TTL_SECONDS, what every shipped preset sets (`scripts/deploy.sh:41`): the
        // RedStone push feeds run a 12 h heartbeat, so 6 h would mark a healthy feed stale
        // for half of every interval.
        redstone_ttl: 86_400,
    }
}

/// deJTRSY-like: 18 dp RWA on Sushi with RedStone only and the 250 bps spread.
pub fn cfg_sushi18() -> Cfg {
    Cfg {
        rwa_decimals: 18,
        venue: VenueKind::Sushi,
        oracles: OracleSet::RedStoneOnly,
        max_swap: 200_000 * ONE,
        max_invested: 8_000_000 * ONE,
        max_spread_bps: 250,
        slippage_bps: 100,
        sep40_ttl: 900,
        redstone_ttl: 86_400, // MAX_TTL_SECONDS, as in every shipped preset (see `cfg_aqua7`)
    }
}

pub struct Fx {
    pub env: Env,
    pub cfg: Cfg,
    pub usdc: Address,
    pub rwa: Address,
    pub admin: Address,
    pub keeper: Address,
    pub sep40: Address,
    pub redstone: Address,
    pub aqua_pool: Address,
    pub sushi_pool: Address,
    pub venue: Venue,
    pub oracles: Vec<OracleSource>,
    pub strategy_id: Address,
}

impl Fx {
    pub fn strategy(&self) -> RwaYieldSharingStrategyClient<'_> {
        RwaYieldSharingStrategyClient::new(&self.env, &self.strategy_id)
    }
    pub fn sep40(&self) -> MockSep40Client<'_> {
        MockSep40Client::new(&self.env, &self.sep40)
    }
    pub fn redstone(&self) -> MockRedStoneClient<'_> {
        MockRedStoneClient::new(&self.env, &self.redstone)
    }
    pub fn aqua(&self) -> MockAquaPoolClient<'_> {
        MockAquaPoolClient::new(&self.env, &self.aqua_pool)
    }
    pub fn sushi(&self) -> MockSushiPoolClient<'_> {
        MockSushiPoolClient::new(&self.env, &self.sushi_pool)
    }
    pub fn rwa_token(&self) -> MockTokenClient<'_> {
        MockTokenClient::new(&self.env, &self.rwa)
    }
    /// One whole RWA in raw units.
    pub fn one_rwa(&self) -> i128 {
        10i128.pow(self.cfg.rwa_decimals)
    }
    pub fn config(&self) -> Config {
        Config { asset: self.usdc.clone(), rwa_asset: self.rwa.clone(), asset_decimals: 7, rwa_decimals: self.cfg.rwa_decimals, venue: self.venue.clone() }
    }
}

pub fn now(f: &Fx) -> u64 {
    f.env.ledger().timestamp()
}

pub fn setup(cfg: Cfg) -> Fx {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START_TS);

    let usdc = env.register_stellar_asset_contract_v2(Address::generate(&env)).address();
    let rwa = env.register(MockToken, (cfg.rwa_decimals,));
    let one_rwa = 10i128.pow(cfg.rwa_decimals);

    // Aqua: tokens [usdc, rwa] -> usdc_idx 0, rwa_idx 1. Sushi: token0 = rwa (as deJTRSY).
    let tokens: Vec<Address> = soroban_sdk::vec![&env, usdc.clone(), rwa.clone()];
    let aqua_pool = env.register(MockAquaPool, (tokens, usdc.clone(), cfg.rwa_decimals, PRICE_ONE));
    let sushi_pool = env.register(MockSushiPool, (rwa.clone(), usdc.clone(), usdc.clone(), cfg.rwa_decimals, PRICE_ONE));
    for pool in [&aqua_pool, &sushi_pool] {
        StellarAssetClient::new(&env, &usdc).mint(pool, &(100_000_000 * ONE));
        MockTokenClient::new(&env, &rwa).mint(pool, &(100_000_000 * one_rwa));
    }

    let sep40 = env.register(MockSep40, (SEP40_DECIMALS, PRICE_ONE * 10_000_000, START_TS));
    let redstone = env.register(MockRedStone, (REDSTONE_DECIMALS as u64, (PRICE_ONE * 10) as u128, START_TS * 1_000));

    let sep40_src = OracleSource { kind: OracleKind::Sep40(sep40.clone(), rwa.clone()), ttl_seconds: cfg.sep40_ttl, decimals: SEP40_DECIMALS };
    let redstone_src = OracleSource { kind: OracleKind::RedStone(redstone.clone()), ttl_seconds: cfg.redstone_ttl, decimals: REDSTONE_DECIMALS };
    let oracles: Vec<OracleSource> = match cfg.oracles {
        OracleSet::RedStoneOnly => soroban_sdk::vec![&env, redstone_src],
        OracleSet::Both => soroban_sdk::vec![&env, sep40_src, redstone_src],
    };
    let venue = match cfg.venue {
        VenueKind::Aqua => Venue::Aqua(aqua_pool.clone(), 1, 0),
        VenueKind::Sushi => Venue::Sushi(sushi_pool.clone(), true),
    };

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let init_args: Vec<soroban_sdk::Val> = (
        rwa.clone(),
        admin.clone(),
        keeper.clone(),
        oracles.clone(),
        venue.clone(),
        cfg.max_swap,
        cfg.max_invested,
        cfg.max_spread_bps,
        cfg.slippage_bps,
    )
        .into_val(&env);
    let strategy_id = env.register(RwaYieldSharingStrategy, (usdc.clone(), init_args));

    Fx { env, cfg, usdc, rwa, admin, keeper, sep40, redstone, aqua_pool, sushi_pool, venue, oracles, strategy_id }
}

/// Mints `amount` USDC to `who` and allowlists it (admin auth is mocked).
pub fn fund(f: &Fx, who: &Address, amount: i128) {
    StellarAssetClient::new(&f.env, &f.usdc).mint(who, &amount);
    f.strategy().add_depositor(who);
}

/// Sets every source to the same price (USDC stroops per whole RWA, 7 dp), timestamped now.
pub fn set_all_prices(f: &Fx, price: i128) {
    let t = now(f);
    f.sep40().set_price(&(price * 10_000_000), &t);
    f.redstone().set_price(&((price * 10) as u128), &(t * 1_000));
    f.aqua().set_price(&price);
    f.sushi().set_price(&price);
}

/// Moves the ledger clock forward `secs` and re-stamps every source at `price`, so oracle
/// TTLs do not fire. One call per `ANCHOR_WINDOW_SECONDS` lets the anchor move one spread
/// step; one call of `TIMELOCK_SECONDS` makes a proposal executable.
pub fn advance(f: &Fx, secs: u64, price: i128) {
    f.env.ledger().with_mut(|l| l.timestamp += secs);
    set_all_prices(f, price);
}

/// Test-only: writes `Params.otc_max_deviation_bps` directly, bypassing the admin path
/// (`propose_params` / `execute_params` / `set_otc_max_deviation_bps`, all present). Kept because the
/// OTC fill tests (`src/test_otc.rs`) want a band in place without driving the 72 h proposal
/// flow first.
pub fn set_otc_max_deviation(f: &Fx, bps: u32) {
    f.env.as_contract(&f.strategy_id, || {
        let mut p = crate::storage::get_params(&f.env).unwrap();
        p.otc_max_deviation_bps = bps;
        crate::storage::set_params(&f.env, &p);
    });
}

pub fn rwa_held(f: &Fx) -> i128 {
    f.rwa_token().balance(&f.strategy_id)
}

pub fn idle(f: &Fx) -> i128 {
    TokenClient::new(&f.env, &f.usdc).balance(&f.strategy_id)
}

/// Puts RWA into the strategy outside any flow (pricing tests before deposit exists).
pub fn mint_rwa_to_strategy(f: &Fx, qty: i128) {
    f.rwa_token().mint(&f.strategy_id, &qty);
}

/// Share-math values (balance, deposit/withdraw returns) carry virtual-offset rounding.
pub fn assert_near(actual: i128, expected: i128, tol: i128) {
    assert!((actual - expected).abs() <= tol, "expected ~{} (±{}), got {}", expected, tol, actual);
}

/// Wasm code hash of a deployed test contract (the hash-allowlist key).
pub fn wasm_hash(c: &Address) -> soroban_sdk::BytesN<32> {
    match c.executable() {
        Some(soroban_sdk::Executable::Wasm(h)) => h,
        other => panic!("expected a wasm contract, got {:?}", other),
    }
}
