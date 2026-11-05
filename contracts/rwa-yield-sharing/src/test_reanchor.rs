#![cfg(test)]
//! `reanchor_to_oracle`: the admin steps the anchor down toward a confirmed
//! oracle drop, at most `MAX_REANCHOR_STEP_BPS` per call and one call per window, instead of
//! letting the clamp book it one `max_spread_bps` step per window.
extern crate std;

use soroban_sdk::testutils::{Address as _, Events, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, IntoVal};

use crate::constants::{ANCHOR_WINDOW_SECONDS, MAX_RATE, MAX_REANCHOR_STEP_BPS, SCALAR_12};
use crate::pricing::PricingMode;
use crate::math;
use crate::storage;
use crate::testutils::*;
use crate::StrategyError;

const R1: i128 = 10_000_000 * SCALAR_12; // 1.00
const P80: i128 = 8_000_000; // 0.80 USDC per RWA, a 20% impairment
const R80: i128 = P80 * SCALAR_12;
/// One full `MAX_REANCHOR_STEP_BPS` (500) under `R1`: where a single reanchor lands on a 20% drop.
const R95: i128 = 9_500_000 * SCALAR_12;

fn last_valid(f: &Fx) -> Option<i128> {
    f.env.as_contract(&f.strategy_id, || storage::get_last_valid_price(&f.env))
}

/// 1_000 USDC deposited and all of it invested at 1.00, so the anchor is 1.00 and NAV is
/// entirely held RWA.
fn invested(cfg: Cfg) -> (Fx, Address) {
    let f = setup(cfg);
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 1_000 * ONE);
    f.strategy().deposit(&(1_000 * ONE), &vault);
    f.strategy().invest(&(1_000 * ONE));
    (f, vault)
}

#[test]
fn the_clamp_books_a_real_drop_one_spread_per_window_and_reanchor_books_one_capped_step() {
    for cfg in [cfg_aqua7(), cfg_sushi18()] {
        let (f, vault) = invested(cfg);
        let s = cfg.max_spread_bps as i128;
        set_all_prices(&f, P80);
        // Without the admin: one spread step now, one more per window.
        assert_eq!(f.strategy().get_pricing_state().rate, R1 * (10_000 - s) / 10_000);
        assert_near(f.strategy().balance(&vault), 1_000 * ONE * (10_000 - s) / 10_000, 10);

        // A 2000 bps gap: the reanchor moves the anchor one capped 500 bps step, and the rate
        // sits one spread under the new anchor, as the clamp always prices.
        let new_anchor = f.strategy().reanchor_to_oracle(&R80);
        assert_eq!(new_anchor, R95);
        assert_eq!(last_valid(&f), Some(R95));
        let st = f.strategy().get_pricing_state();
        assert_eq!((st.mode, st.rate, st.raw_rate), (PricingMode::Live, R95 * (10_000 - s) / 10_000, R80));
        assert_near(f.strategy().balance(&vault), 950 * ONE * (10_000 - s) / 10_000, 10);
    }
}

#[test]
fn reanchor_emits_one_event_with_the_old_new_and_expected_rates() {
    use crate::events::ReanchorEvent;
    use soroban_sdk::{Event as _, String};
    let (f, _) = invested(cfg_aqua7());
    set_all_prices(&f, P80);
    let expected = 8_010_000 * SCALAR_12;
    f.strategy().reanchor_to_oracle(&expected);
    let want = ReanchorEvent {
        strategy_name: String::from_str(&f.env, crate::constants::STRATEGY_NAME),
        old_rate: R1,
        new_rate: R95,
        expected_rate: expected,
    };
    let all = f.env.events().all();
    assert_eq!(all.events().len(), 1);
    assert_eq!(all.events()[0], want.to_xdr(&f.env, &f.strategy_id));
}

#[test]
fn reanchor_is_admin_only() {
    let (f, _) = invested(cfg_aqua7());
    set_all_prices(&f, P80);
    let stranger = Address::generate(&f.env);
    for who in [&f.keeper, &stranger] {
        f.env.mock_auths(&[MockAuth {
            address: who,
            invoke: &MockAuthInvoke {
                contract: &f.strategy_id,
                fn_name: "reanchor_to_oracle",
                args: (R80,).into_val(&f.env),
                sub_invokes: &[],
            },
        }]);
        // `Err(Err(_))`: the host refused the missing admin signature; a contract error
        // (`Err(Ok(_))`) would mean the call got past the auth check.
        assert!(matches!(f.strategy().try_reanchor_to_oracle(&R80), Err(Err(_))));
        assert_eq!(last_valid(&f), Some(R1), "nothing written");
    }
    f.env.mock_auths(&[MockAuth {
        address: &f.admin,
        invoke: &MockAuthInvoke {
            contract: &f.strategy_id,
            fn_name: "reanchor_to_oracle",
            args: (R80,).into_val(&f.env),
            sub_invokes: &[],
        },
    }]);
    assert_eq!(f.strategy().reanchor_to_oracle(&R80), R95);
}

#[test]
fn reanchor_only_moves_the_anchor_down() {
    let (f, _) = invested(cfg_aqua7());
    // Oracle above the anchor: refused, so the admin can never inflate NAV this way.
    set_all_prices(&f, 10_500_000);
    let r105 = 10_500_000 * SCALAR_12;
    assert_eq!(f.strategy().try_reanchor_to_oracle(&r105), Err(Ok(StrategyError::InvalidArgument)));
    // Oracle at the anchor: nothing to book.
    set_all_prices(&f, PRICE_ONE);
    assert_eq!(f.strategy().try_reanchor_to_oracle(&R1), Err(Ok(StrategyError::InvalidArgument)));
    assert_eq!(last_valid(&f), Some(R1));
}

#[test]
fn reanchor_needs_the_admins_figure_to_match_the_oracle() {
    let (f, _) = invested(cfg_aqua7());
    set_all_prices(&f, P80);
    // The multisig signed 0.90; the oracle reads 0.80 (moved, or wrong): refused.
    assert_eq!(f.strategy().try_reanchor_to_oracle(&(9_000_000 * SCALAR_12)), Err(Ok(StrategyError::InvalidArgument)));
    // Inside one `max_spread_bps` (1% here) of the oracle: accepted, anchored at the oracle.
    assert_eq!(f.strategy().reanchor_to_oracle(&(8_050_000 * SCALAR_12)), R95);
    // The tolerance is `spread_bps <= max_spread_bps` (100 here), measured against the larger
    // rate and floored: 0.808_0808 is 99.99 bps above 0.80 and passes, 0.8082 is 101 and not.
    let (g, _) = invested(cfg_aqua7());
    set_all_prices(&g, P80);
    assert_eq!(g.strategy().try_reanchor_to_oracle(&(8_082_000 * SCALAR_12)), Err(Ok(StrategyError::InvalidArgument)));
    assert_eq!(g.strategy().reanchor_to_oracle(&(8_080_808 * SCALAR_12)), R95);
    // Out-of-range figures.
    assert_eq!(f.strategy().try_reanchor_to_oracle(&0), Err(Ok(StrategyError::InvalidArgument)));
    assert_eq!(f.strategy().try_reanchor_to_oracle(&(MAX_RATE + 1)), Err(Ok(StrategyError::InvalidArgument)));
}

#[test]
fn reanchor_needs_every_configured_oracle_live_and_agreeing() {
    // Disagreeing: Reflector-shaped feed at 0.80, RedStone at 0.90. The clamp prices at the
    // min, but a reanchor needs every source inside one spread of the admin's figure.
    let (f, _) = invested(cfg_aqua7());
    set_all_prices(&f, P80);
    f.redstone().set_price(&((9_000_000 * 10) as u128), &(now(&f) * 1_000));
    assert_eq!(f.strategy().try_reanchor_to_oracle(&R80), Err(Ok(StrategyError::InvalidArgument)));

    // Stale: RedStone live at 0.80, the other source past its TTL. The mode is still `Live`
    // (one live source), but the reanchor needs the full set.
    let (g, _) = invested(cfg_aqua7());
    set_all_prices(&g, P80);
    g.sep40().set_price(&(P80 * 10_000_000), &(now(&g) - g.cfg.sep40_ttl - 1));
    assert_eq!(g.strategy().get_pricing_state().mode, PricingMode::Live);
    assert_eq!(g.strategy().try_reanchor_to_oracle(&R80), Err(Ok(StrategyError::ExternalError)));
    assert_eq!(last_valid(&g), Some(R1));
}

#[test]
fn reanchor_is_refused_in_oracle_down_and_while_frozen() {
    let (f, _) = invested(cfg_aqua7());
    set_all_prices(&f, P80);
    f.sep40().set_panic(&true);
    f.redstone().set_panic(&true);
    assert_eq!(f.strategy().get_pricing_state().mode, PricingMode::OracleDown);
    assert_eq!(f.strategy().try_reanchor_to_oracle(&R80), Err(Ok(StrategyError::ExternalError)));

    // Keeper freeze, then admin freeze: a freeze says the oracle is not to be trusted, so
    // the admin unfreezes first if it now trusts it.
    for by_admin in [false, true] {
        let (g, _) = invested(cfg_aqua7());
        set_all_prices(&g, P80);
        let who = if by_admin { g.admin.clone() } else { g.keeper.clone() };
        g.strategy().freeze_price(&who, &true);
        assert_eq!(g.strategy().try_reanchor_to_oracle(&R80), Err(Ok(StrategyError::ExternalError)));
        assert_eq!(last_valid(&g), Some(R1));
        g.strategy().freeze_price(&g.admin, &false);
        assert_eq!(g.strategy().reanchor_to_oracle(&R80), R95);
    }
}

#[test]
fn the_clamp_restarts_from_the_new_anchor() {
    let (f, _) = invested(cfg_aqua7());
    set_all_prices(&f, P80);
    f.strategy().reanchor_to_oracle(&R80);
    // A further, wrong drop to 0.40 moves the rate one spread under the NEW anchor ...
    set_all_prices(&f, 4_000_000);
    assert_eq!(f.strategy().get_pricing_state().rate, R95 * 99 / 100);
    // ... the reanchor started a fresh window, so a guarded call in it leaves the anchor ...
    f.strategy().harvest(&f.keeper, &None);
    assert_eq!(last_valid(&f), Some(R95));
    // ... and the next window moves it one step, as before.
    advance(&f, ANCHOR_WINDOW_SECONDS, 4_000_000);
    f.strategy().harvest(&f.keeper, &None);
    assert_eq!(last_valid(&f), Some(R95 * 99 / 100));
    // Recovery is bounded the same way: upward one spread per window, never a reanchor.
    set_all_prices(&f, PRICE_ONE);
    assert_eq!(f.strategy().get_pricing_state().rate, R95 * 99 / 100 * 101 / 100);
}

#[test]
fn a_gap_up_to_the_cap_books_in_one_call() {
    // Exactly 500 bps: the capped step and the oracle coincide.
    let (f, _) = invested(cfg_aqua7());
    set_all_prices(&f, 9_500_000);
    assert_eq!(f.strategy().reanchor_to_oracle(&R95), R95);
    assert_eq!(f.strategy().get_pricing_state().rate, R95);
    // 300 bps: under the cap, so the anchor lands on the oracle, not on the step.
    let (g, _) = invested(cfg_aqua7());
    set_all_prices(&g, 9_700_000);
    let r97 = 9_700_000 * SCALAR_12;
    assert_eq!(g.strategy().reanchor_to_oracle(&r97), r97);
    assert_eq!(g.strategy().get_pricing_state().rate, r97);
}

#[test]
fn a_second_reanchor_in_the_same_window_is_refused() {
    let (f, _) = invested(cfg_aqua7());
    set_all_prices(&f, P80);
    f.strategy().reanchor_to_oracle(&R80);
    advance(&f, ANCHOR_WINDOW_SECONDS - 1, P80);
    assert_eq!(f.strategy().try_reanchor_to_oracle(&R80), Err(Ok(StrategyError::InvalidArgument)));
    assert_eq!(last_valid(&f), Some(R95), "nothing written");
    advance(&f, 1, P80);
    assert_eq!(f.strategy().reanchor_to_oracle(&R80), R95 * 95 / 100);
}

#[test]
fn a_2000_bps_gap_books_in_capped_steps_one_window_apart() {
    // The step is 500 bps of the CURRENT anchor, so it compounds: 0.95^4 = 0.8145 is still
    // above 0.80 and the fifth call lands on the oracle. No guarded call runs in between, so
    // only the reanchors move the anchor.
    let (f, _) = invested(cfg_aqua7());
    set_all_prices(&f, P80);
    let mut want = R1;
    for call in 1..=5 {
        if call > 1 {
            advance(&f, ANCHOR_WINDOW_SECONDS, P80);
        }
        want = math::apply_bps_ceil(want, -(MAX_REANCHOR_STEP_BPS as i128)).unwrap().max(R80);
        assert_eq!(f.strategy().reanchor_to_oracle(&R80), want, "call {call}");
    }
    assert_eq!(want, R80);
    assert_eq!(last_valid(&f), Some(R80));
    // Nothing left to book.
    advance(&f, ANCHOR_WINDOW_SECONDS, P80);
    assert_eq!(f.strategy().try_reanchor_to_oracle(&R80), Err(Ok(StrategyError::InvalidArgument)));
}

#[test]
fn the_clamp_step_and_one_reanchor_are_the_most_one_window_can_move() {
    // The reanchor restarts the anchor window, and its own cooldown is that same window, so
    // the most the anchor can fall within any hour is one clamp write plus one reanchor.
    let (f, _) = invested(cfg_aqua7());
    set_all_prices(&f, P80);
    f.strategy().reanchor_to_oracle(&R80);
    advance(&f, ANCHOR_WINDOW_SECONDS, P80);
    f.strategy().harvest(&f.keeper, &None);
    let after_clamp = R95 * 99 / 100;
    assert_eq!(last_valid(&f), Some(after_clamp));
    let stepped = math::apply_bps_ceil(after_clamp, -500).unwrap();
    assert_eq!(f.strategy().reanchor_to_oracle(&R80), stepped);
    assert_eq!(f.strategy().try_reanchor_to_oracle(&R80), Err(Ok(StrategyError::InvalidArgument)));
    f.strategy().harvest(&f.keeper, &None);
    assert_eq!(last_valid(&f), Some(stepped), "the clamp waits a window after the reanchor");
}

#[test]
fn the_capped_step_rounds_up_so_it_never_exceeds_500_bps() {
    // An anchor whose 95% is not whole: flooring would move it a hair past 500 bps.
    let (f, _) = invested(cfg_sushi18());
    set_all_prices(&f, P80);
    let odd = R1 + 1;
    f.env.as_contract(&f.strategy_id, || storage::set_anchor(&f.env, odd, now(&f)));
    let got = f.strategy().reanchor_to_oracle(&R80);
    assert_eq!(got, 9_500_000_000_000_000_001);
    assert!(got * 10_000 >= odd * 9_500);
    assert!((got - 1) * 10_000 < odd * 9_500, "the fewest units that stay within the cap");
}
