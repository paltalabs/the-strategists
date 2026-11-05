#![cfg(test)]
extern crate std;

use soroban_sdk::{testutils::Address as _, Address, IntoVal, Vec};

use crate::storage::{OracleKind, OracleSource, Venue};
use crate::testutils::*;
use crate::{StableBondWrapperStrategy, StrategyError};

/// Registers a strategy with `init_args` overridden field by field. Returns Err on panic.
fn try_construct(f: &Fx, oracles: Vec<OracleSource>, venue: Venue, max_swap: i128, max_invested: i128, spread: u32, slippage: u32) -> std::thread::Result<Address> {
    let env = f.env.clone();
    let args: Vec<soroban_sdk::Val> = (
        f.rwa.clone(), f.admin.clone(), f.keeper.clone(), oracles, venue, max_swap, max_invested, spread, slippage,
    )
        .into_val(&env);
    let usdc = f.usdc.clone();
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || env.register(StableBondWrapperStrategy, (usdc, args))))
}

#[test]
fn constructs_aqua_with_two_oracles_and_caches_decimals() {
    let f = setup(cfg_aqua7());
    let p = f.strategy().get_params();
    assert_eq!(p.admin, f.admin);
    assert_eq!(p.keeper, f.keeper);
    assert_eq!(p.pending_admin, None);
    assert_eq!(p.oracles.len(), 2);
    assert_eq!(f.strategy().get_config().venue, f.venue);
    assert_eq!(p.max_swap, 10_000 * ONE);
    assert!(!p.paused);
    assert!(!p.auto_invest);
    assert_eq!(p.otc_max_deviation_bps, 0);
    assert_eq!(p.frozen_until, 0);
    assert!(!p.keeper_freeze_spent);
    assert_eq!(f.strategy().asset(), f.usdc);
}

#[test]
fn constructs_sushi_with_redstone_only_and_18dp_rwa() {
    let f = setup(cfg_sushi18());
    let p = f.strategy().get_params();
    assert_eq!(p.oracles.len(), 1);
    assert_eq!(f.strategy().get_config().venue, Venue::Sushi(f.sushi_pool.clone(), true));
}

#[test]
fn rejects_zero_and_four_oracles() {
    let f = setup(cfg_aqua7());
    let none: Vec<OracleSource> = soroban_sdk::vec![&f.env];
    assert!(try_construct(&f, none, f.venue.clone(), ONE, ONE, 100, 100).is_err());
    let src = f.oracles.get(0).unwrap();
    let four = soroban_sdk::vec![&f.env, src.clone(), src.clone(), src.clone(), src.clone()];
    assert!(try_construct(&f, four, f.venue.clone(), ONE, ONE, 100, 100).is_err());
}

#[test]
fn rejects_ttl_out_of_bounds_and_wrong_decimals() {
    let f = setup(cfg_aqua7());
    let mut src = f.oracles.get(0).unwrap();
    src.ttl_seconds = 0;
    assert!(try_construct(&f, soroban_sdk::vec![&f.env, src.clone()], f.venue.clone(), ONE, ONE, 100, 100).is_err());
    src.ttl_seconds = 86_401;
    assert!(try_construct(&f, soroban_sdk::vec![&f.env, src.clone()], f.venue.clone(), ONE, ONE, 100, 100).is_err());
    src.ttl_seconds = 900;
    src.decimals = 7; // live feed says 14
    assert!(try_construct(&f, soroban_sdk::vec![&f.env, src], f.venue.clone(), ONE, ONE, 100, 100).is_err());
}

#[test]
fn rejects_unreachable_oracle() {
    let f = setup(cfg_aqua7());
    let src = OracleSource { kind: OracleKind::RedStone(Address::generate(&f.env)), ttl_seconds: 900, decimals: 8 };
    assert!(try_construct(&f, soroban_sdk::vec![&f.env, src], f.venue.clone(), ONE, ONE, 100, 100).is_err());
}

#[test]
fn rejects_wrong_venue_token_order() {
    let f = setup(cfg_aqua7());
    // Aqua indices swapped, Sushi rwa_is_token0 wrong.
    assert!(try_construct(&f, f.oracles.clone(), Venue::Aqua(f.aqua_pool.clone(), 0, 1), ONE, ONE, 100, 100).is_err());
    assert!(try_construct(&f, f.oracles.clone(), Venue::Aqua(f.aqua_pool.clone(), 1, 1), ONE, ONE, 100, 100).is_err());
    assert!(try_construct(&f, f.oracles.clone(), Venue::Sushi(f.sushi_pool.clone(), false), ONE, ONE, 100, 100).is_err());
    // And the correct Sushi venue is accepted on the same fixture.
    assert!(try_construct(&f, f.oracles.clone(), Venue::Sushi(f.sushi_pool.clone(), true), ONE, ONE, 100, 100).is_ok());
}

#[test]
fn rejects_out_of_range_knobs() {
    let f = setup(cfg_aqua7());
    let o = f.oracles.clone();
    let v = f.venue.clone();
    assert!(try_construct(&f, o.clone(), v.clone(), 0, ONE, 100, 100).is_err()); // max_swap
    assert!(try_construct(&f, o.clone(), v.clone(), ONE, 0, 100, 100).is_err()); // max_invested
    assert!(try_construct(&f, o.clone(), v.clone(), ONE, ONE, 9, 100).is_err()); // spread low
    assert!(try_construct(&f, o.clone(), v.clone(), ONE, ONE, 501, 100).is_err()); // spread high (cap 500)
    assert!(try_construct(&f, o.clone(), v.clone(), ONE, ONE, 100, 501).is_err()); // slippage
}

#[test]
fn allowlists_are_admin_only_and_pause_is_admin_or_keeper() {
    let f = setup(cfg_aqua7());
    let vault = Address::generate(&f.env);
    // mock_all_auths authorizes whoever the contract asks for, so role checks that compare
    // addresses are what we test here; pure require_auth-only methods are covered by the
    // real-auth integration suite (Task 18).
    f.strategy().add_depositor(&vault);
    assert!(f.strategy().is_depositor(&vault));
    f.strategy().remove_depositor(&vault);
    assert!(!f.strategy().is_depositor(&vault));

    let stranger = Address::generate(&f.env);
    assert_eq!(f.strategy().try_pause_deposits(&stranger, &true), Err(Ok(StrategyError::NotAuthorized)));
    f.strategy().pause_deposits(&f.keeper, &true);
    assert!(f.strategy().get_params().paused);
    f.strategy().pause_deposits(&f.admin, &false);
    assert!(!f.strategy().get_params().paused);
}

#[test]
fn otc_counterparty_view_defaults_false() {
    let f = setup(cfg_aqua7());
    assert!(!f.strategy().is_otc_counterparty(&Address::generate(&f.env)));
}

#[test]
fn rejects_token_decimals_above_the_cap() {
    // The cached decimals feed every later `pow10`, whose own guard admits n <= 36, so the
    // constructor's MAX_DECIMALS bound is the only thing keeping a 19 dp token off the
    // contract. Each leg gets its own pool so the venue check passes and the decimals assert
    // is the only difference between them.
    let f = setup(cfg_aqua7());
    let construct_with = |rwa_decimals: u32| {
        let env = f.env.clone();
        let rwa = env.register(MockToken, (rwa_decimals,));
        let tokens: Vec<Address> = soroban_sdk::vec![&env, f.usdc.clone(), rwa.clone()];
        let pool = env.register(MockAquaPool, (tokens, f.usdc.clone(), rwa_decimals, PRICE_ONE));
        let args: Vec<soroban_sdk::Val> = (
            rwa, f.admin.clone(), f.keeper.clone(), f.oracles.clone(), Venue::Aqua(pool, 1, 0), ONE, ONE, 100u32, 100u32, ONE,
        )
            .into_val(&env);
        let usdc = f.usdc.clone();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || env.register(StableBondWrapperStrategy, (usdc, args))))
    };
    assert!(construct_with(18).is_ok(), "18 dp is at the cap and must construct");
    assert!(construct_with(19).is_err(), "19 dp is over MAX_DECIMALS and must abort the deploy");
}
