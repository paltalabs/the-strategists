#![cfg(test)]
//! Real-WASM suite: the strategy against the compiled Aquarius stack, Reflector, the Sushi
//! pool and a RedStone adapter pinned in `external-contracts/`. Auth is real (no
//! `mock_all_auths`) in the role tests. ABIs verified with `stellar contract info interface`.

extern crate std;

use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke},
    token::{StellarAssetClient, TokenClient},
    Address, BytesN, Env, IntoVal, String, Symbol, Vec, U256,
};

use crate::storage::{OracleKind, OracleSource, Venue};
use crate::{StableBondWrapperStrategy, StableBondWrapperStrategyClient, StrategyError};

mod reflector {
    soroban_sdk::contractimport!(file = "../../external-contracts/reflector_oracle.wasm");
}
mod aqua_router {
    soroban_sdk::contractimport!(file = "../../external-contracts/soroban_liquidity_pool_router_contract.wasm");
}
mod aqua_pool {
    soroban_sdk::contractimport!(file = "../../external-contracts/soroban_liquidity_pool_contract.wasm");
}
mod aqua_plane {
    soroban_sdk::contractimport!(file = "../../external-contracts/soroban_liquidity_pool_plane_contract.wasm");
}
mod aqua_calc {
    soroban_sdk::contractimport!(file = "../../external-contracts/soroban_liquidity_pool_liquidity_calculator_contract.wasm");
}
mod aqua_config_storage {
    soroban_sdk::contractimport!(file = "../../external-contracts/soroban_config_storage_contract.wasm");
}
mod aqua_locker_feed {
    soroban_sdk::contractimport!(file = "../../external-contracts/soroban_locker_feed_contract.wasm");
}
mod aqua_token {
    soroban_sdk::contractimport!(file = "../../external-contracts/soroban_token_contract.wasm");
}
mod aqua_gauge {
    soroban_sdk::contractimport!(file = "../../external-contracts/soroban_rewards_gauge_contract.wasm");
}
mod sushi {
    //! `contractimport!` is not usable on this pinned wasm: its spec declares
    //! `SwapStepResult.sqrt_ratio_next: SqrtPriceX96` but never exports `SqrtPriceX96`, so the
    //! generated bindings do not compile (`error[E0425]: cannot find type SqrtPriceX96 in this
    //! scope`). The bytes are pinned by sha256 instead, and the fixture reaches the three entry
    //! points it needs through `Env::invoke_contract` and the crate's own `SushiPoolClient`.
    pub const WASM: &[u8] = soroban_sdk::contractfile!(
        file = "../../external-contracts/sushi_pool.wasm",
        sha256 = "003710b383f9da7d650a7f719a7be479110266427817ebbed61d924505fcd7c7"
    );
}
mod redstone {
    soroban_sdk::contractimport!(file = "../../external-contracts/redstone_price_adapter.wasm", sha256 = "3e464b6dda26a8d5b0bf13d2cb7781a6142bf2c2e4b64edcba093911fa16df5c");
}

const ONE: i128 = 1_0000000;
const ORACLE_DECIMALS: u32 = 14;
/// Reflector stores period/resolution/set_price timestamps in MILLISECONDS.
const ORACLE_RESOLUTION_MS: u32 = 300_000;

#[contract]
pub struct MockVault;

#[contractimpl]
impl MockVault {
    pub fn __constructor(_e: Env) {}
    pub fn go_deposit(e: Env, strategy: Address, amount: i128) -> i128 {
        StableBondWrapperStrategyClient::new(&e, &strategy).deposit(&amount, &e.current_contract_address())
    }
}

fn sac(env: &Env) -> Address {
    env.register_stellar_asset_contract_v2(Address::generate(env)).address()
}

// --- Reflector -------------------------------------------------------------

fn deploy_reflector(env: &Env, rwa: &Address) -> (Address, reflector::Client<'static>) {
    let admin = Address::generate(env);
    let base = Address::generate(env);
    let id = env.register(reflector::WASM, ());
    let client = reflector::Client::new(env, &id);
    client.config(&reflector::ConfigData {
        admin,
        assets: soroban_sdk::vec![env, reflector::Asset::Stellar(rwa.clone())],
        base_asset: reflector::Asset::Stellar(base),
        decimals: ORACLE_DECIMALS,
        period: 86_400_000,
        resolution: ORACLE_RESOLUTION_MS,
    });
    (id, client)
}

/// `price` is USD per RWA at 14 dp; the timestamp is ledger seconds -> ms aligned to resolution.
fn set_reflector_price(env: &Env, oracle: &reflector::Client, price: i128) {
    let res = ORACLE_RESOLUTION_MS as u64;
    let aligned = (env.ledger().timestamp() * 1000 / res) * res;
    oracle.set_price(&soroban_sdk::vec![env, price], &aligned);
}

// --- Aquarius stack (Aquarius AMM testutils) ---------

struct Aqua {
    router: aqua_router::Client<'static>,
    reward_token: Address,
}

fn deploy_aqua(env: &Env) -> Aqua {
    let admin = Address::generate(env);
    let emergency_admin = Address::generate(env);
    let rewards_admin = Address::generate(env);
    let operations_admin = Address::generate(env);
    let pause_admin = Address::generate(env);
    let emergency_pause_admin = Address::generate(env);
    let system_fee_admin = Address::generate(env);
    let payment_to = Address::generate(env);
    let reward_token = sac(env);
    let reward_boost_token = sac(env);

    let router_id = env.register(aqua_router::WASM, ());
    let router = aqua_router::Client::new(env, &router_id);
    let config_storage = env.register(aqua_config_storage::WASM, (admin.clone(), emergency_admin.clone()));
    let boost_feed = env.register(aqua_locker_feed::WASM, (admin.clone(), operations_admin.clone(), emergency_pause_admin.clone()));
    router.init_admin(&admin);
    router.init_config_storage(&admin, &config_storage);
    router.set_privileged_addrs(&admin, &rewards_admin, &operations_admin, &pause_admin, &soroban_sdk::vec![env, emergency_pause_admin.clone()], &system_fee_admin);
    let pool_hash = env.deployer().upload_contract_wasm(aqua_pool::WASM);
    let token_hash = env.deployer().upload_contract_wasm(aqua_token::WASM);
    let gauge_hash = env.deployer().upload_contract_wasm(aqua_gauge::WASM);
    router.set_pool_hash(&admin, &pool_hash);
    router.set_token_hash(&admin, &token_hash);
    // init_standard_pool reads reward_token, reward_boost_token and reward_boost_feed and
    // panics (501) if any is unset, so all three are configured before any pool exists.
    router.set_reward_token(&admin, &reward_token);
    router.configure_init_pool_payment(&admin, &reward_token, &(ONE as u128), &(ONE as u128), &(ONE as u128), &payment_to);
    router.set_reward_boost_config(&admin, &reward_boost_token, &boost_feed);
    router.set_protocol_fee_fraction(&admin, &5000);
    router.pool_gauge_set_reward_thresholds(&admin, &(10 * ONE as u128), &(60 * 60 * 24));
    router.set_rewards_gauge_hash(&admin, &gauge_hash);
    let plane = env.register(aqua_plane::WASM, ());
    router.set_pools_plane(&admin, &plane);
    let calc = env.register(aqua_calc::WASM, ());
    let calc_client = aqua_calc::Client::new(env, &calc);
    calc_client.init_admin(&admin);
    calc_client.set_pools_plane(&admin, &plane);
    router.set_liquidity_calculator(&admin, &calc);
    Aqua { router, reward_token }
}

fn sorted_pair(env: &Env, a: Address, b: Address) -> Vec<Address> {
    if a < b { soroban_sdk::vec![env, a, b] } else { soroban_sdk::vec![env, b, a] }
}

/// Constant-product pool at 0.3% fee. Returns (tokens, pool_index, pool_address).
fn create_pool(env: &Env, aqua: &Aqua, usdc: &Address, rwa: &Address) -> (Vec<Address>, BytesN<32>, Address) {
    let tokens = sorted_pair(env, usdc.clone(), rwa.clone());
    let creator = Address::generate(env);
    StellarAssetClient::new(env, &aqua.reward_token).mint(&creator, &ONE);
    let (pool_index, pool_addr) = aqua.router.init_standard_pool(&creator, &tokens, &30);
    (tokens, pool_index, pool_addr)
}

fn seed_pool(env: &Env, aqua: &Aqua, tokens: &Vec<Address>, pool_index: &BytesN<32>, usdc: &Address, rwa: &Address, amt: u128) {
    let lp = Address::generate(env);
    StellarAssetClient::new(env, usdc).mint(&lp, &(amt as i128));
    StellarAssetClient::new(env, rwa).mint(&lp, &(amt as i128));
    aqua.router.deposit(&lp, tokens, pool_index, &soroban_sdk::vec![env, amt, amt], &0u128);
}

// --- Sushi pool ------------------------------------------------------------

/// The pinned pool reads its protocol fee from its factory on every quote and every swap, so
/// a factory has to exist: with a plain address there the call dies as
/// `Error(Storage, MissingValue)` "trying to get non-existing value for contract instance".
/// Zero protocol fee, which is what the mainnet pool runs.
#[contract]
pub struct SushiFactory;

#[contractimpl]
impl SushiFactory {
    pub fn __constructor(_e: Env) {}
    pub fn get_protocol_fee_0(_e: Env) -> u32 {
        0
    }
    pub fn get_protocol_fee_1(_e: Env) -> u32 {
        0
    }
}

/// sqrt(1.0) * 2^96 for a 1:1 price with equal decimals.
fn sqrt_price_one(env: &Env) -> U256 {
    U256::from_parts(env, 0, 0, 1 << 32, 0) // 2^96 = 2^(64+32)
}

/// Constructor (factory, token0, token1, fee, tick_spacing, flash_executor, admin) ->
/// `initialize(2^96)` -> `mint` full range at `liquidity`. All three land against the pinned
/// mainnet wasm; `initialize` and `mint` go through `invoke_contract` because the spec cannot
/// be imported (see `mod sushi`). Fee tier 500 (0.05%), tick spacing 10, so -887_270/887_270
/// is the widest tick-spaced range.
fn deploy_sushi_pool(env: &Env, token0: &Address, token1: &Address, liquidity: u128) -> Address {
    let id = env.register(
        sushi::WASM,
        (env.register(SushiFactory, ()), token0.clone(), token1.clone(), 500u32, 10i32, Address::generate(env), Address::generate(env)),
    );
    env.invoke_contract::<()>(
        &id,
        &Symbol::new(env, "initialize"),
        soroban_sdk::vec![env, sqrt_price_one(env).into_val(env)],
    );
    let lp = Address::generate(env);
    StellarAssetClient::new(env, token0).mint(&lp, &((liquidity * 2) as i128));
    StellarAssetClient::new(env, token1).mint(&lp, &((liquidity * 2) as i128));
    let hints = crate::abi::SushiPoolClient::new(env, &id).get_oracle_hints();
    env.invoke_contract::<(u128, u128)>(
        &id,
        &Symbol::new(env, "mint"),
        soroban_sdk::vec![
            env,
            lp.into_val(env),
            lp.into_val(env),
            (-887_270i32).into_val(env),
            887_270i32.into_val(env),
            liquidity.into_val(env),
            hints.into_val(env)
        ],
    );
    id
}

// --- RedStone adapter --------------------------------------------------------

fn deploy_redstone_adapter(env: &Env) -> Address {
    let id = env.register(redstone::WASM, ());
    redstone::Client::new(env, &id).init(&Address::generate(env), &String::from_str(env, "USTRY"));
    id
}

// --- Strategy fixture --------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum RealVenue {
    Aqua,
    Sushi,
}

struct Fx {
    env: Env,
    usdc: Address,
    rwa: Address,
    admin: Address,
    keeper: Address,
    oracle: reflector::Client<'static>,
    pool: Address,
    strategy_id: Address,
}

impl Fx {
    fn strategy(&self) -> StableBondWrapperStrategyClient<'_> {
        StableBondWrapperStrategyClient::new(&self.env, &self.strategy_id)
    }
}

/// Every fixture step (Reflector config, Aqua admin wiring, allowlisting) needs auth, so the
/// env starts with `mock_all_auths()`; the role test switches to explicit `mock_auths` after.
fn setup_real(venue: RealVenue, pool_depth: u128) -> Fx {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();
    env.ledger().set_timestamp(1_000_000);
    let usdc = sac(&env);
    let rwa = sac(&env);
    let (oracle_id, oracle) = deploy_reflector(&env, &rwa);
    set_reflector_price(&env, &oracle, 10i128.pow(ORACLE_DECIMALS));

    let (pool, venue_val) = match venue {
        RealVenue::Aqua => {
            let aqua = deploy_aqua(&env);
            let (tokens, pool_index, pool_addr) = create_pool(&env, &aqua, &usdc, &rwa);
            seed_pool(&env, &aqua, &tokens, &pool_index, &usdc, &rwa, pool_depth);
            let rwa_idx = tokens.first_index_of(rwa.clone()).unwrap();
            let usdc_idx = tokens.first_index_of(usdc.clone()).unwrap();
            (pool_addr.clone(), Venue::Aqua(pool_addr, rwa_idx, usdc_idx))
        }
        RealVenue::Sushi => {
            let (t0, t1) = if rwa < usdc { (rwa.clone(), usdc.clone()) } else { (usdc.clone(), rwa.clone()) };
            let addr = deploy_sushi_pool(&env, &t0, &t1, pool_depth);
            (addr.clone(), Venue::Sushi(addr, t0 == rwa))
        }
    };

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let oracles: Vec<OracleSource> = soroban_sdk::vec![
        &env,
        OracleSource { kind: OracleKind::Sep40(oracle_id, rwa.clone()), ttl_seconds: 900, decimals: ORACLE_DECIMALS }
    ];
    let init_args: Vec<soroban_sdk::Val> = (
        rwa.clone(), admin.clone(), keeper.clone(), oracles, venue_val,
        10_000 * ONE, 1_000_000 * ONE, 100u32, 100u32,
    )
        .into_val(&env);
    let strategy_id = env.register(StableBondWrapperStrategy, (usdc.clone(), init_args));
    Fx { env, usdc, rwa, admin, keeper, oracle, pool, strategy_id }
}

fn fund(f: &Fx, who: &Address, amount: i128) {
    StellarAssetClient::new(&f.env, &f.usdc).mint(who, &amount);
    f.strategy().add_depositor(who);
}

fn rwa_held(f: &Fx) -> i128 {
    TokenClient::new(&f.env, &f.rwa).balance(&f.strategy_id)
}

/// Pushes the Aqua pool spot: a fresh trader swaps `amount` of `token_in` into the pool.
/// Selling USDC in moves the RWA price up; selling RWA in moves it down. On a 1M/1M pool a
/// 25_000 USDC buy lands the spot at (1.025)^2 ≈ 1.05.
fn move_pool_spot(f: &Fx, token_in: &Address, amount: i128) {
    let trader = Address::generate(&f.env);
    StellarAssetClient::new(&f.env, token_in).mint(&trader, &amount);
    let pool = aqua_pool::Client::new(&f.env, &f.pool);
    let tokens = pool.get_tokens();
    let in_idx = tokens.first_index_of(token_in.clone()).unwrap();
    let out_idx = if in_idx == 0 { 1 } else { 0 };
    pool.swap(&trader, &in_idx, &out_idx, &(amount as u128), &1u128);
}

#[test]
fn aqua_direct_swap_auth_and_full_cycle() {
    // Permanent form of the depth-2 auth spike: deposit, keeper invest (direct pool swap with one
    // transfer grant), appreciation, withdraw with a shortfall sale.
    let f = setup_real(RealVenue::Aqua, 1_000_000 * ONE as u128);
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 20_000 * ONE);
    f.strategy().deposit(&(20_000 * ONE), &vault);
    // The gate rate is the oracle's 1.00, so `min_rwa_out` for a tranche X is 0.99*X while
    // the pool fills 0.997*X/(1 + X/1e6). The 1% floor therefore caps a tranche on this book
    // at ~7_070 USDC; 2_000 fills at 1_990.03 against a floor of 1_980.
    let got = f.strategy().invest(&(2_000 * ONE));
    assert!(got > 1_985 * ONE && got < 2_000 * ONE, "0.3% fee + impact, got {}", got);
    assert_eq!(rwa_held(&f), got);
    set_reflector_price(&f.env, &f.oracle, 105 * 10i128.pow(12)); // 1.05
    move_pool_spot(&f, &f.usdc, 25_000 * ONE); // pool follows the oracle up to ~1.05
    // `Live` is clamped to the anchor band and the anchor moves at most one spread per
    // `ANCHOR_WINDOW_SECONDS`, so a +5% move is booked over six windows, never at once. Each
    // pass re-stamps the feed and calls a state-changing method so the anchor can step.
    for _ in 0..8 {
        f.env.ledger().with_mut(|l| l.timestamp += crate::constants::ANCHOR_WINDOW_SECONDS);
        set_reflector_price(&f.env, &f.oracle, 105 * 10i128.pow(12));
        f.strategy().harvest(&f.keeper, &None);
    }
    assert_eq!(f.strategy().get_pricing_state().mode, crate::PricingMode::Live);
    let bal = f.strategy().balance(&vault);
    assert!(bal > 20_000 * ONE, "appreciation shows in balance, got {}", bal);
    // 19_000 out: 18_000 idle + a 1_000 shortfall sold with min_out = 1_000
    f.strategy().withdraw(&(19_000 * ONE), &vault, &vault);
    assert_eq!(TokenClient::new(&f.env, &f.usdc).balance(&vault), 19_000 * ONE);
    assert!(rwa_held(&f) < got, "the shortfall came from an RWA sale");
}

#[test]
fn pool_spot_manipulation_moves_neither_nav_nor_the_gates() {
    let f = setup_real(RealVenue::Aqua, 1_000_000 * ONE as u128);
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 20_000 * ONE);
    f.strategy().deposit(&(20_000 * ONE), &vault);
    f.strategy().invest(&(2_000 * ONE));
    let before = f.strategy().balance(&vault);
    move_pool_spot(&f, &f.rwa, 400_000 * ONE); // dump RWA: spot ≈ 0.51, a -49% push
    let s = f.strategy().get_pricing_state();
    assert_eq!(s.mode, crate::PricingMode::Live);
    assert_eq!(f.strategy().balance(&vault), before, "the pool is not a pricing input");
    // A shortfall sale into the dumped pool gives up at most the oracle-implied RWA plus 1%,
    // so it reverts rather than selling at the dump.
    assert_eq!(f.strategy().try_withdraw(&(19_000 * ONE), &vault, &vault), Err(Ok(StrategyError::InternalSwapError)));
    // Buying from the dumped pool clears the oracle floor with room to spare: accretive.
    let held = rwa_held(&f);
    let got = f.strategy().invest(&(1_000 * ONE));
    assert!(got > 1_500 * ONE, "~2x the oracle-implied quantity, got {}", got);
    assert_eq!(rwa_held(&f), held + got);
    fund(&f, &vault, ONE);
    f.strategy().deposit(&ONE, &vault);
}

#[test]
fn stale_reflector_degrades_to_oracle_down_and_balance_still_returns() {
    let f = setup_real(RealVenue::Aqua, 1_000_000 * ONE as u128);
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 20_000 * ONE);
    f.strategy().deposit(&(20_000 * ONE), &vault);
    f.strategy().invest(&(2_000 * ONE)); // the 1% floor caps a tranche at ~7_070 here
    f.env.ledger().with_mut(|l| l.timestamp += 1_000); // past the 900 s TTL
    let s = f.strategy().get_pricing_state();
    assert_eq!(s.mode, crate::PricingMode::OracleDown);
    assert!(f.strategy().balance(&vault) > 19_000 * ONE);
    // 19_000 out: 18_000 idle + a 1_000 shortfall sold at the anchor; an honest pool fills it.
    f.strategy().withdraw(&(19_000 * ONE), &vault, &vault);
    assert_eq!(TokenClient::new(&f.env, &f.usdc).balance(&vault), 19_000 * ONE);
}

#[test]
fn withdraw_reverts_on_execution_slippage_against_a_thin_pool() {
    // 200k pool against a 1.00 feed. The 1% floor closes the pool in both directions: a 2_000
    // buy needs 1_980 RWA and the pool fills 1_974.31 (0.3% fee + ~1% impact), and a 5_000
    // shortfall sale sends at most 5_050 RWA for ~4_911 USDC against a min_out of 5_000.
    let f = setup_real(RealVenue::Aqua, 200_000 * ONE as u128);
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 5_000 * ONE);
    f.strategy().deposit(&(5_000 * ONE), &vault);
    assert_eq!(f.strategy().get_pricing_state().mode, crate::PricingMode::Live);
    assert_eq!(f.strategy().try_invest(&(2_000 * ONE)), Err(Ok(StrategyError::InternalSwapError)));
    // RWA bought while the book was deep; only the thin book is left to sell it into.
    StellarAssetClient::new(&f.env, &f.rwa).mint(&f.strategy_id, &(10_000 * ONE));
    // idle 5_000, shortfall 5_000: the sale falls short of its min_out, so withdraw reverts
    // instead of short-paying.
    assert_eq!(f.strategy().try_withdraw(&(10_000 * ONE), &vault, &vault), Err(Ok(StrategyError::InternalSwapError)));
    // Idle-only withdrawal goes through.
    f.strategy().withdraw(&(4_000 * ONE), &vault, &vault);
}

#[test]
fn inflation_attack_resisted_real() {
    let f = setup_real(RealVenue::Aqua, 1_000_000 * ONE as u128);
    let attacker = Address::generate(&f.env);
    let victim = Address::generate(&f.env);
    fund(&f, &attacker, 10_000 * ONE);
    fund(&f, &victim, 1_000 * ONE);
    f.strategy().deposit(&1, &attacker);
    StellarAssetClient::new(&f.env, &f.rwa).mint(&f.strategy_id, &(5_000 * ONE)); // donation
    let v = f.strategy().deposit(&(1_000 * ONE), &victim);
    assert!(v >= 999 * ONE, "got {}", v);
}

#[test]
fn deposit_via_allowlisted_wasm_hash_real_stack() {
    let f = setup_real(RealVenue::Aqua, 1_000_000 * ONE as u128);
    // A vault CONTRACT calls deposit, so the strategy's USDC pull is authorized by a contract
    // inside the call stack rather than by the root invoker: plain `mock_all_auths` rejects it
    // with `Error(Auth, InvalidAction)`, exactly as in the mock suite.
    f.env.mock_all_auths_allowing_non_root_auth();
    let vault_id = f.env.register(MockVault, ());
    StellarAssetClient::new(&f.env, &f.usdc).mint(&vault_id, &(100 * ONE));
    f.strategy().add_allowed_hash(&crate::testutils::wasm_hash(&vault_id));
    assert_eq!(MockVaultClient::new(&f.env, &vault_id).go_deposit(&f.strategy_id, &(100 * ONE)), 100 * ONE);
}

#[test]
fn roles_are_enforced_with_real_auth() {
    // After setup, `env.mock_auths` replaces mock-all: each call then carries exactly the
    // auth listed for it.
    let f = setup_real(RealVenue::Aqua, 1_000_000 * ONE as u128);
    let bot2 = Address::generate(&f.env);
    let stranger = Address::generate(&f.env);
    let hash = BytesN::from_array(&f.env, &[7u8; 32]);

    // --- state setup, still under the fixture's mock-all, so every negative below fails on
    // --- the signature and nothing else: each call would succeed for the right signer.
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 20_000 * ONE);
    f.strategy().deposit(&(20_000 * ONE), &vault);
    f.strategy().invest(&(2_000 * ONE)); // held RWA, so divest and divest_otc have stock
    f.strategy().add_depositor(&stranger); // something for remove_depositor to remove
    f.strategy().add_allowed_hash(&hash); // something for remove_allowed_hash to remove
    let live_oracles = f.strategy().get_params().oracles;
    f.strategy().propose_oracles(&live_oracles);
    let cp = Address::generate(&f.env);
    f.strategy().propose_otc_counterparty(&cp);
    // A knob proposal matured by the same jump, carrying `setup_real`'s own live values (`:303`):
    // what the table tries below is the signer, not the effect, so executing it moves nothing the
    // rest of this test reads. It has to be proposed here because re-stamping the reflector past a
    // second wait needs the reflector admin's auth, which the `mock_auths` region below withdraws.
    f.strategy().propose_params(&100, &100, &(10_000 * ONE), &0);
    f.env.ledger().with_mut(|l| l.timestamp += crate::constants::TIMELOCK_SECONDS);
    set_reflector_price(&f.env, &f.oracle, 10i128.pow(ORACLE_DECIMALS)); // re-stamp past the wait
    // The counterparty's side of an OTC fill, so only the signer is left to prove.
    let expiry = f.env.ledger().sequence() + 100_000;
    StellarAssetClient::new(&f.env, &f.rwa).mint(&cp, &(1_000 * ONE));
    StellarAssetClient::new(&f.env, &f.usdc).mint(&cp, &(1_000 * ONE));
    TokenClient::new(&f.env, &f.rwa).approve(&cp, &f.strategy_id, &(1_000 * ONE), &expiry);
    TokenClient::new(&f.env, &f.usdc).approve(&cp, &f.strategy_id, &(1_000 * ONE), &expiry);

    // --- from here every call carries exactly one signature ---
    let auth = |who: &Address, fn_name: &'static str, args: Vec<soroban_sdk::Val>| {
        f.env.mock_auths(&[MockAuth {
            address: who,
            invoke: &MockAuthInvoke { contract: &f.strategy_id, fn_name, args, sub_invokes: &[] },
        }]);
    };

    // keeper signing an admin method fails
    auth(&f.keeper, "set_keeper", (bot2.clone(),).into_val(&f.env));
    assert!(f.strategy().try_set_keeper(&bot2).is_err());
    // admin signing succeeds
    auth(&f.admin, "set_keeper", (bot2.clone(),).into_val(&f.env));
    f.strategy().set_keeper(&bot2);
    // admin signing a keeper method fails
    auth(&f.admin, "invest", (ONE,).into_val(&f.env));
    assert!(f.strategy().try_invest(&ONE).is_err());
    // the rotated keeper signing it succeeds
    auth(&bot2, "invest", (ONE,).into_val(&f.env));
    assert!(f.strategy().invest(&ONE) > 0);
    // divest is keeper-only too
    auth(&f.admin, "divest", (100 * ONE,).into_val(&f.env));
    assert!(f.strategy().try_divest(&(100 * ONE)).is_err());
    auth(&bot2, "divest", (100 * ONE,).into_val(&f.env));
    assert!(f.strategy().divest(&(100 * ONE)) >= 100 * ONE);

    // the four allowlist setters gate on `require_admin` alone: no address compare backs them up
    auth(&bot2, "add_depositor", (stranger.clone(),).into_val(&f.env));
    assert!(f.strategy().try_add_depositor(&stranger).is_err());
    auth(&f.admin, "add_depositor", (stranger.clone(),).into_val(&f.env));
    f.strategy().add_depositor(&stranger);
    auth(&stranger, "remove_depositor", (stranger.clone(),).into_val(&f.env));
    assert!(f.strategy().try_remove_depositor(&stranger).is_err());
    auth(&f.admin, "remove_depositor", (stranger.clone(),).into_val(&f.env));
    f.strategy().remove_depositor(&stranger);
    assert!(!f.strategy().is_depositor(&stranger));
    auth(&bot2, "add_allowed_hash", (hash.clone(),).into_val(&f.env));
    assert!(f.strategy().try_add_allowed_hash(&hash).is_err());
    auth(&f.admin, "add_allowed_hash", (hash.clone(),).into_val(&f.env));
    f.strategy().add_allowed_hash(&hash);
    auth(&stranger, "remove_allowed_hash", (hash.clone(),).into_val(&f.env));
    assert!(f.strategy().try_remove_allowed_hash(&hash).is_err());
    auth(&f.admin, "remove_allowed_hash", (hash.clone(),).into_val(&f.env));
    f.strategy().remove_allowed_hash(&hash);
    assert!(!f.strategy().is_allowed_hash(&hash));

    // the timelocked oracle set: propose / execute / cancel are all admin-only
    auth(&bot2, "execute_oracles", soroban_sdk::vec![&f.env]);
    assert!(f.strategy().try_execute_oracles().is_err());
    auth(&f.admin, "execute_oracles", soroban_sdk::vec![&f.env]);
    assert_eq!(f.strategy().execute_oracles(), 1);
    auth(&stranger, "propose_oracles", (live_oracles.clone(),).into_val(&f.env));
    assert!(f.strategy().try_propose_oracles(&live_oracles).is_err());
    auth(&f.admin, "propose_oracles", (live_oracles.clone(),).into_val(&f.env));
    f.strategy().propose_oracles(&live_oracles);
    auth(&bot2, "cancel_oracles", soroban_sdk::vec![&f.env]);
    assert!(f.strategy().try_cancel_oracles().is_err());
    auth(&f.admin, "cancel_oracles", soroban_sdk::vec![&f.env]);
    f.strategy().cancel_oracles();
    assert!(f.strategy().get_pending_oracles().is_none());

    // the timelocked price knobs: propose / execute / cancel are all admin-only
    let knobs: Vec<soroban_sdk::Val> = (100u32, 100u32, 10_000 * ONE, 0u32).into_val(&f.env);
    auth(&bot2, "execute_params", soroban_sdk::vec![&f.env]);
    assert!(f.strategy().try_execute_params().is_err());
    auth(&f.admin, "execute_params", soroban_sdk::vec![&f.env]);
    f.strategy().execute_params();
    auth(&stranger, "propose_params", knobs.clone());
    assert!(f.strategy().try_propose_params(&100, &100, &(10_000 * ONE), &0).is_err());
    auth(&f.admin, "propose_params", knobs);
    f.strategy().propose_params(&100, &100, &(10_000 * ONE), &0);
    auth(&bot2, "cancel_params", soroban_sdk::vec![&f.env]);
    assert!(f.strategy().try_cancel_params().is_err());
    auth(&f.admin, "cancel_params", soroban_sdk::vec![&f.env]);
    f.strategy().cancel_params();
    assert!(f.strategy().get_pending_params().is_none());
    // narrowing the OTC band is immediate, and admin-only too. It ships `0`, so the admin's
    // call takes the equal-is-not-a-raise branch.
    auth(&bot2, "set_otc_max_deviation_bps", (0u32,).into_val(&f.env));
    assert!(f.strategy().try_set_otc_max_deviation_bps(&0).is_err());
    auth(&f.admin, "set_otc_max_deviation_bps", (0u32,).into_val(&f.env));
    f.strategy().set_otc_max_deviation_bps(&0);

    // the timelocked OTC counterparty, and the keeper-only fills it unlocks
    auth(&bot2, "execute_otc_counterparty", (cp.clone(),).into_val(&f.env));
    assert!(f.strategy().try_execute_otc_counterparty(&cp).is_err());
    auth(&f.admin, "execute_otc_counterparty", (cp.clone(),).into_val(&f.env));
    f.strategy().execute_otc_counterparty(&cp);
    assert!(f.strategy().is_otc_counterparty(&cp));
    // The fill price is the gated rate itself (the band ships `0`); a view needs no signature.
    let px = f.strategy().get_pricing_state().rate;
    auth(&f.admin, "invest_otc", (cp.clone(), 100 * ONE, px).into_val(&f.env));
    assert!(f.strategy().try_invest_otc(&cp, &(100 * ONE), &px).is_err());
    auth(&bot2, "invest_otc", (cp.clone(), 100 * ONE, px).into_val(&f.env));
    assert!(f.strategy().invest_otc(&cp, &(100 * ONE), &px) > 0);
    let px = f.strategy().get_pricing_state().rate;
    auth(&stranger, "divest_otc", (cp.clone(), 100 * ONE, px).into_val(&f.env));
    assert!(f.strategy().try_divest_otc(&cp, &(100 * ONE), &px).is_err());
    auth(&bot2, "divest_otc", (cp.clone(), 100 * ONE, px).into_val(&f.env));
    assert!(f.strategy().divest_otc(&cp, &(100 * ONE), &px) > 0);
    // keeper signing a timelocked admin proposal fails (Task 15's admin gate)
    auth(&f.keeper, "propose_otc_counterparty", (bot2.clone(),).into_val(&f.env));
    assert!(f.strategy().try_propose_otc_counterparty(&bot2).is_err());
    auth(&bot2, "propose_otc_counterparty", (stranger.clone(),).into_val(&f.env));
    assert!(f.strategy().try_propose_otc_counterparty(&stranger).is_err());
    auth(&f.admin, "propose_otc_counterparty", (stranger.clone(),).into_val(&f.env));
    f.strategy().propose_otc_counterparty(&stranger);
    auth(&bot2, "remove_otc_counterparty", (cp.clone(),).into_val(&f.env));
    assert!(f.strategy().try_remove_otc_counterparty(&cp).is_err());
    auth(&f.admin, "remove_otc_counterparty", (cp.clone(),).into_val(&f.env));
    f.strategy().remove_otc_counterparty(&cp);
    assert!(!f.strategy().is_otc_counterparty(&cp));

    // freeze_price: the named caller must sign; keeper may freeze, only the admin may unfreeze
    auth(&stranger, "freeze_price", (bot2.clone(), true).into_val(&f.env));
    assert!(f.strategy().try_freeze_price(&bot2, &true).is_err(), "naming the keeper is not signing as it");
    auth(&stranger, "freeze_price", (stranger.clone(), true).into_val(&f.env));
    assert!(f.strategy().try_freeze_price(&stranger, &true).is_err());
    auth(&bot2, "freeze_price", (bot2.clone(), true).into_val(&f.env));
    f.strategy().freeze_price(&bot2, &true);
    assert!(f.strategy().get_params().frozen_until > 0);
    auth(&bot2, "freeze_price", (bot2.clone(), false).into_val(&f.env));
    assert!(f.strategy().try_freeze_price(&bot2, &false).is_err());
    auth(&bot2, "freeze_price", (bot2.clone(), true).into_val(&f.env));
    assert!(f.strategy().try_freeze_price(&bot2, &true).is_err(), "the keeper's freeze is spent");
    auth(&f.admin, "freeze_price", (f.admin.clone(), false).into_val(&f.env));
    f.strategy().freeze_price(&f.admin, &false);
    assert_eq!(f.strategy().get_params().frozen_until, 0);

    // pending admin must sign accept_admin
    let new_admin = Address::generate(&f.env);
    auth(&f.admin, "transfer_admin", (new_admin.clone(),).into_val(&f.env));
    f.strategy().transfer_admin(&new_admin);
    auth(&f.admin, "accept_admin", soroban_sdk::vec![&f.env]);
    assert!(f.strategy().try_accept_admin().is_err());
    auth(&stranger, "accept_admin", soroban_sdk::vec![&f.env]);
    assert!(f.strategy().try_accept_admin().is_err());
    auth(&new_admin, "accept_admin", soroban_sdk::vec![&f.env]);
    f.strategy().accept_admin();
    assert_eq!(f.strategy().get_params().admin, new_admin);
}

#[test]
fn redstone_adapter_decimals_decode_and_read_failure_degrades() {
    let f = setup_real(RealVenue::Aqua, 1_000_000 * ONE as u128);
    let adapter = deploy_redstone_adapter(&f.env);
    // If decimals() itself reaches for the router and fails here, record that in the module
    // doc and drop the propose/execute half of this test: the adapter is then unusable in
    // tests beyond ABI decoding of its spec, and the mock carries the price path.
    assert_eq!(redstone::Client::new(&f.env, &adapter).decimals(), 8);
    let mut oracles = f.strategy().get_params().oracles;
    oracles.push_back(OracleSource { kind: OracleKind::RedStone(adapter), ttl_seconds: 21_600, decimals: 8 });
    f.strategy().propose_oracles(&oracles); // live decimals() passes
    f.env.ledger().with_mut(|l| l.timestamp += crate::constants::TIMELOCK_SECONDS);
    set_reflector_price(&f.env, &f.oracle, 10i128.pow(ORACLE_DECIMALS)); // re-stamp past the wait
    f.strategy().execute_oracles(); // the still-live Reflector source satisfies the liveness check
    let s = f.strategy().get_pricing_state();
    assert_eq!(s.live_oracles, 1, "the adapter cannot reach its router here: not live, no revert");
    assert_eq!(s.mode, crate::PricingMode::Live);
}

#[test]
fn sushi_real_pool_quote_swap_and_hint_ordering() {
    let f = setup_real(RealVenue::Sushi, 1_000_000 * ONE as u128);
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 20_000 * ONE);
    f.strategy().deposit(&(20_000 * ONE), &vault);
    let got = f.strategy().invest(&(5_000 * ONE));
    // 0.05% fee plus ~0.5% impact: the full-range position is L = 1e13, i.e. ~1M of each
    // token of virtual reserve, so 5_000 in moves the tick from 0 to 19 and fills 4_972.65.
    assert!(got > 4_965 * ONE && got < 5_000 * ONE, "0.05% fee + impact, got {}", got);
    // 17_000 out: 15_000 idle + a 2_000 shortfall sold on the real pool
    f.strategy().withdraw(&(17_000 * ONE), &vault, &vault);
    assert_eq!(TokenClient::new(&f.env, &f.usdc).balance(&vault), 17_000 * ONE);
    // grant-before-hints must fail against the real pool
    let probe = f.env.register(crate::testutils::SushiMisorderedProbe, ());
    StellarAssetClient::new(&f.env, &f.rwa).mint(&probe, &(100 * ONE));
    let p = crate::testutils::SushiMisorderedProbeClient::new(&f.env, &probe);
    let zero_for_one = f.rwa < f.usdc;
    assert!(p.try_swap_grant_before_hints(&f.pool, &f.rwa, &zero_for_one, &(10 * ONE)).is_err());
}

