#![cfg(test)]
extern crate std;

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use crate::testutils::*;
use crate::StrategyError;

fn seeded(f: &Fx, usdc: i128) -> Address {
    let vault = Address::generate(&f.env);
    fund(f, &vault, usdc);
    f.strategy().deposit(&usdc, &vault);
    vault
}

#[test]
fn invest_buys_a_tranche_within_every_cap() {
    let f = setup(cfg_aqua7());
    let vault = seeded(&f, 50_000 * ONE);
    let got = f.strategy().invest(&(10_000 * ONE));
    assert_eq!(got, 10_000 * ONE);
    assert_eq!(idle(&f), 40_000 * ONE);
    assert_eq!(rwa_held(&f), 10_000 * ONE);
    assert_eq!(f.strategy().balance(&vault), 50_000 * ONE);
    // over max_swap
    assert_eq!(f.strategy().try_invest(&(10_001 * ONE)), Err(Ok(StrategyError::InvalidArgument)));
    // over idle
    let g = setup(cfg_aqua7());
    seeded(&g, 100 * ONE);
    assert_eq!(g.strategy().try_invest(&(101 * ONE)), Err(Ok(StrategyError::InsufficientBalance)));
    // non positive
    assert_eq!(g.strategy().try_invest(&0), Err(Ok(StrategyError::OnlyPositiveAmountAllowed)));
}

#[test]
fn invest_respects_max_invested() {
    let f = setup(Cfg { max_invested: 15_000 * ONE, ..cfg_aqua7() });
    seeded(&f, 50_000 * ONE);
    f.strategy().invest(&(10_000 * ONE));
    assert_eq!(f.strategy().try_invest(&(10_000 * ONE)), Err(Ok(StrategyError::InvalidArgument)));
    f.strategy().invest(&(5_000 * ONE));
    assert_eq!(rwa_held(&f), 15_000 * ONE);
}

#[test]
fn invest_refused_when_paused_or_degraded() {
    let f = setup(cfg_aqua7());
    // mock_all_auths satisfies `keeper.require_auth()` for any caller, so the keeper-only
    // gate on `invest` and `divest` is not provable in this suite; it is covered by the
    // real-auth integration suite (Task 18). What this test proves is the pause refusal and
    // the two non-`Live` refusals.
    seeded(&f, 50_000 * ONE);
    f.strategy().pause_deposits(&f.admin, &true);
    assert_eq!(f.strategy().try_invest(&(1_000 * ONE)), Err(Ok(StrategyError::NotAuthorized)));
    f.strategy().pause_deposits(&f.admin, &false);
    f.sep40().set_panic(&true);
    f.redstone().set_panic(&true); // OracleDown
    assert_eq!(f.strategy().try_invest(&(1_000 * ONE)), Err(Ok(StrategyError::ExternalError)));
    f.sep40().set_panic(&false);
    f.redstone().set_panic(&false);
    f.strategy().freeze_price(&f.keeper, &true); // Frozen
    assert_eq!(f.strategy().try_invest(&(1_000 * ONE)), Err(Ok(StrategyError::ExternalError)));
}

#[test]
fn invest_min_out_is_oracle_implied_minus_slippage() {
    let f = setup(cfg_aqua7());
    seeded(&f, 50_000 * ONE);
    f.aqua().set_fill_bps(&9_850); // 1.5% short of the quote: under the 1% floor
    assert_eq!(f.strategy().try_invest(&(1_000 * ONE)), Err(Ok(StrategyError::InternalSwapError)));
    f.aqua().set_fill_bps(&9_950);
    assert_eq!(f.strategy().invest(&(1_000 * ONE)), 995 * ONE);
}

#[test]
fn divest_rebuilds_idle_from_rwa() {
    let f = setup(cfg_aqua7());
    seeded(&f, 50_000 * ONE);
    f.strategy().invest(&(10_000 * ONE));
    let out = f.strategy().divest(&(2_000 * ONE));
    assert_eq!(out, 2_020 * ONE, "sells 2_020 RWA (1% margin) at 1.00");
    assert_eq!(idle(&f), 42_020 * ONE);
    assert_eq!(rwa_held(&f), 7_980 * ONE);
    assert_eq!(f.strategy().try_divest(&(10_001 * ONE)), Err(Ok(StrategyError::InvalidArgument)));
    assert_eq!(f.strategy().try_divest(&0), Err(Ok(StrategyError::OnlyPositiveAmountAllowed)));
}

#[test]
fn divest_works_in_degraded_modes_with_a_rate_but_not_frozen_without_anchor() {
    let f = setup(cfg_aqua7());
    seeded(&f, 50_000 * ONE);
    f.strategy().invest(&(10_000 * ONE));
    f.sep40().set_panic(&true);
    f.redstone().set_panic(&true); // OracleDown: bounded rate, honest pool fills
    assert_eq!(f.strategy().divest(&(1_000 * ONE)), 1_010 * ONE);
    // short fill below the target reverts
    f.aqua().set_fill_bps(&9_000);
    assert_eq!(f.strategy().try_divest(&(1_000 * ONE)), Err(Ok(StrategyError::InternalSwapError)));
}

#[test]
fn divest_clamps_to_held() {
    let f = setup(cfg_aqua7());
    seeded(&f, 50_000 * ONE);
    f.strategy().invest(&(1_000 * ONE));
    // target 5_000 needs 5_050 RWA, only 1_000 held: sells 1_000, pays 1_000 < 5_000: reverts
    assert_eq!(f.strategy().try_divest(&(5_000 * ONE)), Err(Ok(StrategyError::InternalSwapError)));
}

#[test]
fn withdraw_after_invest_uses_the_keeper_buffer() {
    let f = setup(cfg_aqua7());
    let vault = seeded(&f, 50_000 * ONE);
    f.strategy().invest(&(10_000 * ONE)); // idle 40_000, held 10_000
    f.strategy().divest(&(5_000 * ONE)); // sells ceil(5_000 * 1.01) = 5_050 RWA at 1.00
    assert_eq!(idle(&f), 45_050 * ONE, "40_000 left by the invest plus the 5_050 divested");
    assert_eq!(rwa_held(&f), 4_950 * ONE, "10_000 bought minus the 5_050 sold");
    // Venue dead: any shortfall sale would revert, so a withdrawal that still succeeds was
    // served entirely from the keeper-refilled idle buffer.
    f.aqua().set_panic(&true);
    f.strategy().withdraw(&(45_000 * ONE), &vault, &vault);
    assert_eq!(idle(&f), 50 * ONE, "45_050 idle minus the 45_000 shipped");
    assert_eq!(rwa_held(&f), 4_950 * ONE, "the withdraw sold no RWA");
    assert!(f.strategy().balance(&vault) > 0);
}

#[test]
fn divest_fills_exactly_on_a_fair_pool_at_zero_slippage() {
    // Keeper side: same 1-stroop miss as the withdraw shortfall.
    for cfg in [cfg_aqua7(), cfg_sushi18()] {
        let f = setup(Cfg { slippage_bps: 0, ..cfg });
        set_all_prices(&f, 10_745_000);
        seeded(&f, 1_000 * ONE);
        f.strategy().invest(&(1_000 * ONE));
        assert_eq!(idle(&f), 0);
        let out = f.strategy().divest(&ONE);
        assert!(out >= ONE && out <= ONE + 1, "got {}", out);
        assert_eq!(f.strategy().divest(&1), 1, "one stroop: the floor sized it at 0 RWA");
    }
}
