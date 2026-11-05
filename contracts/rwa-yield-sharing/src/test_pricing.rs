#![cfg(test)]
extern crate std;

use soroban_sdk::testutils::{Events, Ledger as _};

use crate::constants::{ANCHOR_WINDOW_SECONDS, MAX_RATE, SCALAR_12};
use crate::pricing::{self, PricingMode};
use crate::storage;
use crate::testutils::*;

const R1: i128 = 10_000_000 * SCALAR_12; // rate at 1.00

fn last_valid(f: &Fx) -> Option<i128> {
    f.env.as_contract(&f.strategy_id, || storage::get_last_valid_price(&f.env))
}

/// Runs the guarded pricing inside the contract frame (what every flow does).
fn guarded(f: &Fx) -> pricing::Nav {
    let config = f.config();
    let params = f.strategy().get_params();
    f.env.as_contract(&f.strategy_id, || pricing::nav_guarded(&f.env, &config, &params).unwrap())
}

fn set_redstone(f: &Fx, price: i128) {
    f.redstone().set_price(&((price * 10) as u128), &(now(f) * 1_000));
}

fn kill_oracles(f: &Fx) {
    f.sep40().set_panic(&true);
    f.redstone().set_panic(&true);
}

#[test]
fn live_takes_the_min_live_oracle() {
    let f = setup(cfg_aqua7());
    set_redstone(&f, 9_950_000); // RedStone 0.995, Reflector-shaped feed 1.00
    let s = f.strategy().get_pricing_state();
    assert_eq!(s.mode, PricingMode::Live);
    assert_eq!(s.rate, 9_950_000 * SCALAR_12);
    assert_eq!(s.raw_rate, 9_950_000 * SCALAR_12);
    assert_eq!(s.live_oracles, 2);
    // The view writes nothing.
    assert_eq!(last_valid(&f), None);
    // The guarded path anchors LastValidPrice.
    let n = guarded(&f);
    assert_eq!(n.state.rate, 9_950_000 * SCALAR_12);
    assert_eq!(last_valid(&f), Some(9_950_000 * SCALAR_12));
}

#[test]
fn the_pool_price_never_moves_the_rate() {
    // The RWA pools sit 100-160 bps under NAV for weeks. Pushed, discounted or dead,
    // the pool changes nothing about the rate.
    let f = setup(cfg_aqua7());
    guarded(&f); // anchor 1.00
    for pool_price in [6_000_000, 9_860_000, 14_000_000] {
        f.aqua().set_price(&pool_price);
        let s = f.strategy().get_pricing_state();
        assert_eq!((s.mode, s.rate), (PricingMode::Live, R1), "pool at {}", pool_price);
    }
    f.aqua().set_panic(&true);
    let s = f.strategy().get_pricing_state();
    assert_eq!((s.mode, s.rate), (PricingMode::Live, R1), "pool dead");
    // Without an anchor too: the raw oracle, not the pool.
    let g = setup(cfg_sushi18());
    g.sushi().set_price(&8_000_000);
    assert_eq!(g.strategy().get_pricing_state().rate, R1);
}

#[test]
fn a_wrong_oracle_moves_the_rate_at_most_one_spread_per_window() {
    let f = setup(cfg_aqua7());
    guarded(&f); // anchor 1.00 at START_TS
    set_all_prices(&f, 1_000_000_000); // every oracle says 100.00
    let s = f.strategy().get_pricing_state();
    assert_eq!(s.mode, PricingMode::Live);
    assert_eq!(s.raw_rate, 1_000_000_000 * SCALAR_12);
    assert_eq!(s.rate, 10_100_000 * SCALAR_12, "clamped to L * (1 + 1%)");
    guarded(&f); // same window: the anchor does not move
    assert_eq!(last_valid(&f), Some(R1));
    advance(&f, ANCHOR_WINDOW_SECONDS, 1_000_000_000);
    guarded(&f); // next window: one spread step
    assert_eq!(last_valid(&f), Some(10_100_000 * SCALAR_12));
    assert_eq!(f.strategy().get_pricing_state().rate, 10_201_000 * SCALAR_12, "1.01 * 1.01");
    // Downward the same: an oracle at 0.01 books at 0.99 first.
    let g = setup(cfg_aqua7());
    guarded(&g);
    set_all_prices(&g, 100_000);
    assert_eq!(g.strategy().get_pricing_state().rate, 9_900_000 * SCALAR_12);
    // A real move inside the band is taken as is.
    let h = setup(cfg_aqua7());
    guarded(&h);
    set_all_prices(&h, 10_004_000); // +4 bps, the largest RedStone write in 90 days
    assert_eq!(h.strategy().get_pricing_state().rate, 10_004_000 * SCALAR_12);
}

#[test]
fn one_low_oracle_lowers_the_rate_inside_the_band() {
    // The min over live oracles: a low source lowers NAV, bounded by L * (1 - s). The
    // presets ship RedStone only, where this cannot happen.
    let f = setup(cfg_aqua7());
    guarded(&f);
    f.sep40().set_price(&(5_000_000 * 10_000_000), &now(&f)); // Reflector-shaped feed at 0.50
    let s = f.strategy().get_pricing_state();
    assert_eq!((s.mode, s.rate), (PricingMode::Live, 9_900_000 * SCALAR_12));
}

#[test]
fn oracle_down_uses_the_anchor_or_zero() {
    let f = setup(cfg_aqua7());
    kill_oracles(&f);
    let s = f.strategy().get_pricing_state();
    assert_eq!((s.mode, s.rate, s.live_oracles), (PricingMode::OracleDown, 0, 0), "no anchor yet");
    f.sep40().set_panic(&false);
    f.redstone().set_panic(&false);
    guarded(&f);
    kill_oracles(&f);
    f.aqua().set_price(&5_000_000); // the pool is irrelevant here too
    let s = f.strategy().get_pricing_state();
    assert_eq!((s.mode, s.rate), (PricingMode::OracleDown, R1));
    guarded(&f);
    assert_eq!(last_valid(&f), Some(R1), "OracleDown never writes the anchor");
}

#[test]
fn frozen_holds_the_anchor_whatever_the_oracles_say() {
    let f = setup(cfg_aqua7());
    guarded(&f); // anchor 1.00
    f.strategy().freeze_price(&f.admin, &true);
    advance(&f, ANCHOR_WINDOW_SECONDS, 10_500_000); // every oracle at 1.05, a window later
    let s = f.strategy().get_pricing_state();
    assert_eq!((s.mode, s.rate, s.live_oracles), (PricingMode::Frozen, R1, 2));
    guarded(&f);
    assert_eq!(last_valid(&f), Some(R1), "Frozen never writes the anchor");
    // Unfrozen, the rate rejoins the oracle one spread step at a time.
    f.strategy().freeze_price(&f.admin, &false);
    let s = f.strategy().get_pricing_state();
    assert_eq!((s.mode, s.rate), (PricingMode::Live, 10_100_000 * SCALAR_12));
    // Frozen with no anchor is 0, like OracleDown.
    let g = setup(cfg_aqua7());
    g.strategy().freeze_price(&g.keeper, &true);
    assert_eq!(g.strategy().get_pricing_state().rate, 0);
}

#[test]
fn stale_oracles_count_as_dead() {
    let f = setup(cfg_aqua7());
    // Past both fixture TTLs: Sep40 900 s, RedStone 86_400 s (`MAX_TTL_SECONDS`, the value
    // every shipped preset uses — `scripts/deploy.sh`).
    f.env.ledger().with_mut(|l| l.timestamp = START_TS + 86_401);
    let s = f.strategy().get_pricing_state();
    assert_eq!(s.live_oracles, 0);
    assert_eq!(s.mode, PricingMode::OracleDown);
}

#[test]
fn nav_is_idle_plus_held_at_rate_for_7dp_and_18dp() {
    let f = setup(cfg_aqua7());
    mint_rwa_to_strategy(&f, 100 * ONE);
    soroban_sdk::token::StellarAssetClient::new(&f.env, &f.usdc).mint(&f.strategy_id, &(5 * ONE));
    set_all_prices(&f, 10_500_000);
    let n = guarded(&f);
    assert_eq!(rwa_held(&f), 100 * ONE);
    assert_eq!(n.idle, 5 * ONE);
    assert_eq!(n.nav, 110 * ONE);

    let g = setup(cfg_sushi18());
    mint_rwa_to_strategy(&g, 100 * ONE_18);
    set_all_prices(&g, 10_367_000);
    let n = guarded(&g);
    assert_eq!(n.state.mode, PricingMode::Live);
    assert_eq!(n.nav, 1_036_700_000);
}

#[test]
fn no_anchor_and_no_oracle_values_held_rwa_at_zero_but_still_returns() {
    let f = setup(cfg_aqua7());
    mint_rwa_to_strategy(&f, 100 * ONE);
    soroban_sdk::token::StellarAssetClient::new(&f.env, &f.usdc).mint(&f.strategy_id, &(5 * ONE));
    kill_oracles(&f);
    let n = guarded(&f);
    assert_eq!(n.nav, 5 * ONE);
}

#[test]
fn guarded_emits_mode_event_when_degraded_and_jump_event_on_a_jump() {
    let f = setup(cfg_aqua7());
    guarded(&f);
    kill_oracles(&f);
    let config = f.config();
    let params = f.strategy().get_params();
    // Drive through a real invocation so events().all() reflects it: harvest prices guarded.
    f.strategy().harvest(&f.keeper, &None);
    let n = f.env.events().all().events().len();
    assert_eq!(n, 2, "PricingModeEvent + SpreadAlertEvent, got {}", n);
    f.sep40().set_panic(&false);
    f.redstone().set_panic(&false);
    set_all_prices(&f, 10_300_000); // +3% vs anchor 1.00 > 1% spread: booked at 1.01, alerted
    f.strategy().harvest(&f.keeper, &None);
    let n = f.env.events().all().events().len();
    assert_eq!(n, 2, "PriceJumpEvent + HarvestEvent, got {}", n);
    assert_eq!(last_valid(&f), Some(R1), "same window: anchor unchanged");
    let _ = (config, params);
}

#[test]
fn harvest_is_keeper_gated_and_moves_no_funds() {
    let f = setup(cfg_aqua7());
    assert_eq!(f.strategy().try_harvest(&f.admin, &None), Err(Ok(crate::StrategyError::NotAuthorized)));
    f.strategy().harvest(&f.keeper, &None);
    assert_eq!(idle(&f), 0);
    assert_eq!(rwa_held(&f), 0);
}

#[test]
fn balance_is_zero_without_shares_and_never_touches_storage() {
    let f = setup(cfg_aqua7());
    assert_eq!(f.strategy().balance(&f.admin), 0);
    kill_oracles(&f);
    assert_eq!(f.strategy().balance(&f.admin), 0);
    assert_eq!(last_valid(&f), None);
}

#[test]
fn an_absurd_oracle_price_cannot_make_balance_revert() {
    // Sep40 reports 14 dp against 7-dp USDC, so `rate = price *
    // 10^(7+12) / 10^14 = price * 10^5`; a price of `4 * MAX_RATE / 10^5` normalizes to
    // ~`4 * MAX_RATE` = 3.4028e34, which fits i128 and which `rate_from_price` returns.
    // Feed that into pricing and the anchor clamp's `apply_bps` or `PriceJumpEvent`'s
    // `spread_bps(3.4028e34, 1e19)` (`|a - b| * 10_000` = 3.4028e38, past i128::MAX =
    // 1.7014e38) would return `Err(ArithmeticError)`, `nav_readonly` would propagate it and
    // `balance()` would revert — a vault-wide DoS from one oracle reading. `MAX_RATE` makes
    // that source not live instead, and pricing falls back to the anchor.
    let f = setup(cfg_aqua7());
    guarded(&f); // anchor 1.00 from a Live resolution
    mint_rwa_to_strategy(&f, 100 * ONE);
    // Seed a vault position so `balance()` prices instead of short-circuiting at zero.
    f.env.as_contract(&f.strategy_id, || {
        storage::set_vault_shares(&f.env, &f.admin, 1_000_000);
        storage::set_total_shares(&f.env, 1_000_000);
    });
    f.sep40().set_price(&(4 * MAX_RATE / 100_000), &now(&f));
    f.redstone().set_panic(&true);
    let s = f.strategy().get_pricing_state();
    assert_eq!(s.live_oracles, 0, "the absurd source is not live");
    assert_eq!(s.mode, PricingMode::OracleDown);
    assert_eq!(s.rate, R1, "the anchor");
    // nav = 100 RWA at 1.00 = 100 USDC = 1e9 stroops, so
    // shares_to_assets_down(1e6, 1e6, 1e9) = floor(1e6 * (1e9 + 1) / (1e6 + VIRTUAL_SHARES))
    //                                      = floor(1_000_000_001 / 2) = 500_000_000.
    assert_eq!(f.strategy().balance(&f.admin), 500_000_000);
}
