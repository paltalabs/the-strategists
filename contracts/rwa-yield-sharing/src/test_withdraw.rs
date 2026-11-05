#![cfg(test)]
extern crate std;

use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::TokenClient;
use soroban_sdk::Address;

use crate::constants::ANCHOR_WINDOW_SECONDS;
use crate::testutils::*;
use crate::StrategyError;

#[test]
fn withdraw_serves_from_idle_without_touching_the_venue() {
    let f = setup(cfg_aqua7());
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 1_000 * ONE);
    f.strategy().deposit(&(1_000 * ONE), &vault);
    f.strategy().invest(&(400 * ONE));
    f.aqua().set_panic(&true); // venue dead: idle still serves
    let bal = f.strategy().withdraw(&(500 * ONE), &vault, &vault);
    assert_eq!(TokenClient::new(&f.env, &f.usdc).balance(&vault), 500 * ONE);
    assert_eq!(bal, 500 * ONE);
    assert_eq!(idle(&f), 100 * ONE);
    assert_eq!(rwa_held(&f), 400 * ONE);
}

#[test]
fn withdraw_sells_only_the_shortfall_and_keeps_surplus_idle() {
    let f = setup(cfg_aqua7());
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 1_000 * ONE);
    f.strategy().deposit(&(1_000 * ONE), &vault);
    f.strategy().invest(&(800 * ONE));
    // idle 200, need 500: sells RWA for 300 (max_rwa_in = 303 at 1% slippage, pays 303)
    f.strategy().withdraw(&(500 * ONE), &vault, &vault);
    assert_eq!(TokenClient::new(&f.env, &f.usdc).balance(&vault), 500 * ONE);
    assert_eq!(idle(&f), 3 * ONE, "surplus from the slippage margin stays idle");
    assert_eq!(rwa_held(&f), 497 * ONE);
}

#[test]
fn withdraw_shortfall_above_max_swap_reverts_with_insufficient_balance() {
    let f = setup(Cfg { max_swap: 100 * ONE, ..cfg_aqua7() });
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 1_000 * ONE);
    f.strategy().deposit(&(1_000 * ONE), &vault);
    for _ in 0..8 {
        f.strategy().invest(&(100 * ONE)); // max_swap is 100 here, so 800 takes eight tranches
    }
    // idle 200, need 500: shortfall 300 > max_swap 100
    assert_eq!(f.strategy().try_withdraw(&(500 * ONE), &vault, &vault), Err(Ok(StrategyError::InsufficientBalance)));
    // 300 (shortfall 100) goes through
    f.strategy().withdraw(&(300 * ONE), &vault, &vault);
}

#[test]
fn withdraw_over_underlying_reverts() {
    let f = setup(cfg_aqua7());
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 1_000 * ONE);
    f.strategy().deposit(&(1_000 * ONE), &vault);
    assert_eq!(f.strategy().try_withdraw(&(1_001 * ONE), &vault, &vault), Err(Ok(StrategyError::InsufficientBalance)));
    assert_eq!(f.strategy().try_withdraw(&0, &vault, &vault), Err(Ok(StrategyError::OnlyPositiveAmountAllowed)));
}

#[test]
fn withdraw_with_shortfall_reverts_when_rate_is_zero_or_venue_is_dead() {
    let f = setup(cfg_aqua7());
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 1_000 * ONE);
    f.strategy().deposit(&(1_000 * ONE), &vault);
    f.strategy().invest(&(800 * ONE));
    // A rate of zero is impossible here (deposit anchored), so kill the venue instead.
    f.aqua().set_panic(&true);
    assert_eq!(f.strategy().try_withdraw(&(500 * ONE), &vault, &vault), Err(Ok(StrategyError::InternalSwapError)));
    // Idle-only still works in the same state.
    f.strategy().withdraw(&(200 * ONE), &vault, &vault);
}

#[test]
fn withdraw_in_oracle_down_uses_the_bounded_rate_and_min_out_is_the_shortfall() {
    let f = setup(cfg_aqua7());
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 1_000 * ONE);
    f.strategy().deposit(&(1_000 * ONE), &vault);
    f.strategy().invest(&(800 * ONE));
    f.sep40().set_panic(&true);
    f.redstone().set_panic(&true);
    // honest pool at 1.00: fills
    f.strategy().withdraw(&(500 * ONE), &vault, &vault);
    assert_eq!(TokenClient::new(&f.env, &f.usdc).balance(&vault), 500 * ONE);
    // pushed pool at 0.60: rate floors at 0.99, rwa_in = 303 of the shortfall, pays 181 < 300: reverts
    f.aqua().set_price(&6_000_000);
    assert_eq!(f.strategy().try_withdraw(&(300 * ONE), &vault, &vault), Err(Ok(StrategyError::InternalSwapError)));
}

#[test]
fn withdraw_is_never_paused_or_allowlist_gated() {
    let f = setup(cfg_aqua7());
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 1_000 * ONE);
    f.strategy().deposit(&(1_000 * ONE), &vault);
    f.strategy().pause_deposits(&f.admin, &true);
    f.strategy().remove_depositor(&vault);
    f.strategy().withdraw(&(1_000 * ONE), &vault, &vault);
    assert_eq!(f.strategy().balance(&vault), 0);
}

#[test]
fn full_withdraw_removes_the_vault_pos_entry() {
    let f = setup(cfg_aqua7());
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 100 * ONE);
    f.strategy().deposit(&(100 * ONE), &vault);
    f.strategy().withdraw(&(100 * ONE), &vault, &vault);
    assert_eq!(f.env.as_contract(&f.strategy_id, || crate::storage::peek_vault_shares(&f.env, &vault)), 0);
    assert_eq!(f.strategy().get_params().max_swap, f.cfg.max_swap); // sanity: params intact
}

#[test]
fn withdraw_realizes_appreciation_as_usdc() {
    let f = setup(cfg_aqua7());
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 1_000 * ONE);
    f.strategy().deposit(&(1_000 * ONE), &vault);
    f.strategy().invest(&(1_000 * ONE)); // 1_000 RWA at 1.00, anchor 1.00
    // The RWA appreciates to 1.10. A real buy happens at the live rate, so the gain has to
    // come after it, and the anchor limiter admits at most one 1% step per window: ten of
    // them clear 1.10 (1.01^10 = 1.1046), each needing a guarded call to rewrite the anchor.
    for _ in 0..10 {
        advance(&f, ANCHOR_WINDOW_SECONDS, 11_000_000);
        f.strategy().harvest(&f.keeper, &None);
    }
    assert_near(f.strategy().balance(&vault), 1_100 * ONE, 10);
    // 1_099 rather than 1_100: the floored underlying sits a few stroops under 1_100.
    f.strategy().withdraw(&(1_099 * ONE), &vault, &vault);
    assert_eq!(TokenClient::new(&f.env, &f.usdc).balance(&vault), 1_099 * ONE);
}

#[test]
fn withdraw_shortfall_into_a_pool_under_nav_reverts_and_idle_still_pays() {
    // The pool 140 bps under NAV (USDY's state) with 1% slippage: the shortfall sale
    // may give up at most the oracle-implied RWA plus 1%, which the pool does not pay for, so
    // it reverts instead of selling holders' RWA at the discount. The keeper refills idle with
    // `divest_otc` at NAV instead.
    let f = setup(cfg_aqua7());
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 1_000 * ONE);
    f.strategy().deposit(&(1_000 * ONE), &vault);
    f.strategy().invest(&(800 * ONE));
    f.aqua().set_price(&9_860_000);
    assert_eq!(f.strategy().try_withdraw(&(500 * ONE), &vault, &vault), Err(Ok(StrategyError::InternalSwapError)));
    f.strategy().withdraw(&(200 * ONE), &vault, &vault);
    assert_eq!(TokenClient::new(&f.env, &f.usdc).balance(&vault), 200 * ONE);
}

#[test]
fn withdraw_while_frozen_pays_at_the_anchor() {
    let f = setup(cfg_aqua7());
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 1_000 * ONE);
    f.strategy().deposit(&(1_000 * ONE), &vault); // anchor 1.00
    f.strategy().freeze_price(&f.keeper, &true);
    set_all_prices(&f, 9_000_000); // oracles and pool at 0.90: the frozen rate ignores them
    assert_near(f.strategy().balance(&vault), 1_000 * ONE, 10);
    f.strategy().withdraw(&(1_000 * ONE), &vault, &vault);
    assert_eq!(TokenClient::new(&f.env, &f.usdc).balance(&vault), 1_000 * ONE);
}

// --- The zero-slippage shortfall sale on a fair pool ---------

/// USDY-like NAV: 1.0745 USDC per whole RWA, a rate whose implied RWA
/// quantities are never whole, so every floor in the sizing shows.
const NAV_10745: i128 = 10_745_000;

/// The shipped presets (`slippage_bps` 0, `scripts/deploy.sh:132`) with every oracle and both
/// fee-free mock pools at `NAV_10745`; 1_000 USDC deposited, 800 of it invested.
fn fair_pool_at_zero_slippage(cfg: Cfg) -> (Fx, Address) {
    let f = setup(Cfg { slippage_bps: 0, ..cfg });
    set_all_prices(&f, NAV_10745);
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 1_000 * ONE);
    f.strategy().deposit(&(1_000 * ONE), &vault);
    f.strategy().invest(&(800 * ONE));
    (f, vault)
}

#[test]
fn withdraw_shortfall_fills_exactly_on_a_fair_pool_at_zero_slippage() {
    // A 1 USDC shortfall at 1.0745 implies 9_306_654.2 raw RWA. Sizing it
    // at the floor sells RWA worth 9_999_999 stroops and misses `min_out` by one; the cap must
    // be the ceiling so a fair, fee-free pool clears the shortfall exactly.
    for cfg in [cfg_aqua7(), cfg_sushi18()] {
        let (f, vault) = fair_pool_at_zero_slippage(cfg);
        let idle_before = idle(&f);
        f.strategy().withdraw(&(idle_before + ONE), &vault, &vault);
        assert_eq!(TokenClient::new(&f.env, &f.usdc).balance(&vault), idle_before + ONE);
        assert!(idle(&f) <= 1, "the sale clears the shortfall with at most one stroop over, got {}", idle(&f));
    }
}

#[test]
fn withdraw_shortfall_boundaries_one_stroop_and_the_whole_position() {
    for cfg in [cfg_aqua7(), cfg_sushi18()] {
        // One stroop of shortfall: the floor sized it at 0 raw RWA and refused outright.
        let (f, vault) = fair_pool_at_zero_slippage(cfg);
        let idle_before = idle(&f);
        f.strategy().withdraw(&(idle_before + 1), &vault, &vault);
        assert_eq!(TokenClient::new(&f.env, &f.usdc).balance(&vault), idle_before + 1);

        // The whole position: the ceiling never asks for more RWA than is held, because the
        // shortfall is at most `value_of(held)`, which is floored.
        let (g, vault) = fair_pool_at_zero_slippage(cfg);
        let all = g.strategy().balance(&vault);
        g.strategy().withdraw(&all, &vault, &vault);
        assert_eq!(TokenClient::new(&g.env, &g.usdc).balance(&vault), all);
        let left_value = rwa_held(&g) * NAV_10745 / g.one_rwa();
        assert!(left_value <= 1, "at most a stroop of RWA left, got {} raw", rwa_held(&g));
    }
}
