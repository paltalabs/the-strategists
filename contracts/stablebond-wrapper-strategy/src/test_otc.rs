#![cfg(test)]
extern crate std;

use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::token::{StellarAssetClient, TokenClient};
use soroban_sdk::{Address, String};

use crate::constants::{MAX_OTC_DEVIATION_BPS, SCALAR_12, STRATEGY_NAME, TIMELOCK_SECONDS};
use crate::events::{OtcDivestEvent, OtcInvestEvent};
use crate::math;
use crate::testutils::*;
use crate::StrategyError;

struct Otc {
    vault: Address,
    mm: Address,
}

/// Compares the `i`-th event of the last top-level invocation against the event the caller
/// expected, the same pattern `src/test_timelock.rs::assert_event_at` uses: `Event::to_xdr`
/// checks every field, not just the count.
fn assert_event_at<E: soroban_sdk::Event>(f: &Fx, i: usize, expected: &E) {
    let all = f.env.events().all();
    assert_eq!(all.events()[i], expected.to_xdr(&f.env, &f.strategy_id), "event #{}", i);
}

/// The gated rate a fill is checked against, read through the same `price_rwa` the fill uses.
/// Prices passed to `invest_otc` / `divest_otc` are in this unit: USDC stroops per whole
/// RWA, times `SCALAR_12`.
fn rate(f: &Fx) -> i128 {
    f.strategy().get_pricing_state().rate
}

/// `rate * (10_000 + bps) / 10_000`, exact for every fixture rate here (all multiples of
/// `SCALAR_12`), so a test can name a price "N bps from the rate" without rounding noise.
fn off(rate: i128, bps: i128) -> i128 {
    rate + rate * bps / 10_000
}

/// Allowlists a counterparty through the 72 h wait with every source at `price` (so the
/// first anchor lands at `price`; see Conventions), then the vault deposits 1M idle. The
/// counterparty holds 1M RWA and 1M USDC and grants allowances.
fn otc_setup(f: &Fx, price: i128) -> Otc {
    let mm = Address::generate(&f.env);
    f.strategy().propose_otc_counterparty(&mm);
    advance(f, TIMELOCK_SECONDS, price);
    f.strategy().execute_otc_counterparty(&mm);
    let vault = Address::generate(&f.env);
    fund(f, &vault, 1_000_000 * ONE);
    f.strategy().deposit(&(1_000_000 * ONE), &vault); // anchors at `price`
    f.rwa_token().mint(&mm, &(1_000_000 * f.one_rwa()));
    StellarAssetClient::new(&f.env, &f.usdc).mint(&mm, &(1_000_000 * ONE));
    f.rwa_token().approve(&mm, &f.strategy_id, &(1_000_000 * f.one_rwa()), &1_000_000);
    TokenClient::new(&f.env, &f.usdc).approve(&mm, &f.strategy_id, &(1_000_000 * ONE), &1_000_000);
    Otc { vault, mm }
}

#[test]
fn invest_otc_pulls_rwa_and_pays_usdc_atomically_at_the_rate() {
    let f = setup(cfg_aqua7());
    let o = otc_setup(&f, 10_500_000); // 1.05
    let r = rate(&f);
    assert_eq!(r, 10_500_000 * SCALAR_12);
    let got = f.strategy().invest_otc(&o.mm, &(105_000 * ONE), &r);
    assert_eq!(got, 100_000 * ONE, "105_000 USDC at 1.05");
    assert_eq!(f.env.events().all().events().len(), 2, "USDC SAC transfer + OtcInvestEvent");
    assert_eq!(rwa_held(&f), got);
    assert_eq!(idle(&f), 895_000 * ONE);
    assert_eq!(TokenClient::new(&f.env, &f.usdc).balance(&o.mm), 1_105_000 * ONE);
    // not bounded by max_swap (10_000): 105_000 went through; NAV unchanged by the fill
    assert_near(f.strategy().balance(&o.vault), 1_000_000 * ONE, 10);
}

#[test]
fn invest_otc_refusals() {
    let f = setup(cfg_aqua7());
    let o = otc_setup(&f, PRICE_ONE);
    let r = rate(&f);
    let stranger = Address::generate(&f.env);
    assert_eq!(f.strategy().try_invest_otc(&stranger, &(1_000 * ONE), &r), Err(Ok(StrategyError::NotAuthorized)));
    // proposed but not executed is not allowed
    let proposed = Address::generate(&f.env);
    f.strategy().propose_otc_counterparty(&proposed);
    assert_eq!(f.strategy().try_invest_otc(&proposed, &(1_000 * ONE), &r), Err(Ok(StrategyError::NotAuthorized)));
    assert_eq!(f.strategy().try_invest_otc(&o.mm, &0, &r), Err(Ok(StrategyError::OnlyPositiveAmountAllowed)));
    assert_eq!(f.strategy().try_invest_otc(&o.mm, &(1_000_001 * ONE), &r), Err(Ok(StrategyError::InsufficientBalance)));
    f.strategy().pause_deposits(&f.keeper, &true);
    assert_eq!(f.strategy().try_invest_otc(&o.mm, &(1_000 * ONE), &r), Err(Ok(StrategyError::NotAuthorized)));
    f.strategy().pause_deposits(&f.keeper, &false);
    // no live oracle: OracleDown and Frozen refuse a buy
    f.sep40().set_panic(&true);
    f.redstone().set_panic(&true);
    assert_eq!(f.strategy().try_invest_otc(&o.mm, &(1_000 * ONE), &r), Err(Ok(StrategyError::ExternalError)));
    f.aqua().set_panic(&true);
    assert_eq!(f.strategy().try_invest_otc(&o.mm, &(1_000 * ONE), &r), Err(Ok(StrategyError::ExternalError)));
    f.sep40().set_panic(&false);
    f.redstone().set_panic(&false);
    f.aqua().set_panic(&false);
    // over max_invested
    let g = setup(Cfg { max_invested: 500 * ONE, ..cfg_aqua7() });
    let og = otc_setup(&g, PRICE_ONE);
    let rg = rate(&g);
    assert_eq!(g.strategy().try_invest_otc(&og.mm, &(501 * ONE), &rg), Err(Ok(StrategyError::InvalidArgument)));
}

#[test]
fn invest_otc_fills_at_the_oracle_rate_whatever_the_pool_and_a_bad_oracle_is_clamped() {
    let f = setup(cfg_aqua7());
    let o = otc_setup(&f, PRICE_ONE); // anchor 1.00
    // Pool 140 bps under NAV, as USDY's sits: the band still hangs off the oracle's 1.00.
    f.aqua().set_price(&9_860_000);
    assert_eq!(rate(&f), PRICE_ONE * SCALAR_12);
    assert_eq!(f.strategy().invest_otc(&o.mm, &(1_000 * ONE), &rate(&f)), 1_000 * ONE);
    // A fill at the pool price is outside a zero band.
    assert_eq!(
        f.strategy().try_invest_otc(&o.mm, &(986 * ONE), &(9_860_000 * SCALAR_12)),
        Err(Ok(StrategyError::InvalidArgument))
    );
    // Pool dead: nothing changes.
    f.aqua().set_panic(&true);
    assert_eq!(f.strategy().invest_otc(&o.mm, &(1_000 * ONE), &rate(&f)), 1_000 * ONE);
    f.aqua().set_panic(&false);
    // Every oracle at 100.00: the rate is clamped to L * (1 + 1%), and a fill priced at the
    // lying oracle is refused: the band hangs off the gated rate, not a source.
    set_all_prices(&f, 1_000_000_000);
    assert_eq!(rate(&f), 10_100_000 * SCALAR_12);
    assert_eq!(
        f.strategy().try_invest_otc(&o.mm, &(1_000 * ONE), &(100 * PRICE_ONE * SCALAR_12)),
        Err(Ok(StrategyError::InvalidArgument))
    );
    // Frozen: invest_otc refuses outright.
    f.strategy().freeze_price(&f.keeper, &true);
    assert_eq!(
        f.strategy().try_invest_otc(&o.mm, &(1_000 * ONE), &rate(&f)),
        Err(Ok(StrategyError::ExternalError))
    );
}

#[test]
fn invest_otc_without_allowance_reverts_and_moves_nothing() {
    let f = setup(cfg_aqua7());
    let o = otc_setup(&f, PRICE_ONE);
    f.rwa_token().approve(&o.mm, &f.strategy_id, &0, &1_000_000); // revoked
    let idle_before = idle(&f);
    assert!(f.strategy().try_invest_otc(&o.mm, &(1_000 * ONE), &rate(&f)).is_err());
    assert_eq!(idle(&f), idle_before);
    assert_eq!(rwa_held(&f), 0);
}

#[test]
fn divest_otc_pulls_usdc_and_pays_rwa_at_the_rate() {
    let f = setup(cfg_aqua7());
    let o = otc_setup(&f, PRICE_ONE);
    let r = rate(&f);
    f.strategy().invest_otc(&o.mm, &(100_000 * ONE), &r); // 100_000 RWA at 1.00
    let mm_rwa_before = f.rwa_token().balance(&o.mm);
    let paid = f.strategy().divest_otc(&o.mm, &(50_000 * ONE), &r);
    assert_eq!(paid, 50_000 * ONE);
    assert_eq!(f.env.events().all().events().len(), 2, "USDC SAC transfer + OtcDivestEvent");
    assert_eq!(f.rwa_token().balance(&o.mm), mm_rwa_before + 50_000 * ONE);
    assert_eq!(idle(&f), 950_000 * ONE);
    assert_eq!(rwa_held(&f), 50_000 * ONE);
    // refused when it would need more RWA than held, and for a stranger
    assert_eq!(f.strategy().try_divest_otc(&o.mm, &(60_000 * ONE), &r), Err(Ok(StrategyError::InsufficientBalance)));
    let stranger = Address::generate(&f.env);
    assert_eq!(f.strategy().try_divest_otc(&stranger, &(1_000 * ONE), &r), Err(Ok(StrategyError::NotAuthorized)));
    assert_eq!(f.strategy().try_divest_otc(&o.mm, &0, &r), Err(Ok(StrategyError::OnlyPositiveAmountAllowed)));
}

#[test]
fn divest_otc_works_degraded_with_a_rate_and_not_frozen_without_anchor() {
    let f = setup(cfg_aqua7());
    let o = otc_setup(&f, PRICE_ONE);
    f.strategy().invest_otc(&o.mm, &(100_000 * ONE), &rate(&f));
    f.sep40().set_panic(&true);
    f.redstone().set_panic(&true);
    f.aqua().set_panic(&true); // OracleDown at anchor 1.00: the exit with nothing live
    assert_eq!(rate(&f), PRICE_ONE * SCALAR_12);
    assert_eq!(f.strategy().divest_otc(&o.mm, &(10_000 * ONE), &rate(&f)), 10_000 * ONE);
    // RWA held before any Live resolution, every source dead: no rate, refused before the
    // price is even looked at.
    let g = setup(cfg_aqua7());
    let mm = Address::generate(&g.env);
    g.strategy().propose_otc_counterparty(&mm);
    advance(&g, TIMELOCK_SECONDS, PRICE_ONE);
    g.strategy().execute_otc_counterparty(&mm);
    mint_rwa_to_strategy(&g, 1_000 * ONE);
    StellarAssetClient::new(&g.env, &g.usdc).mint(&mm, &(1_000 * ONE));
    TokenClient::new(&g.env, &g.usdc).approve(&mm, &g.strategy_id, &(1_000 * ONE), &1_000_000);
    g.sep40().set_panic(&true);
    g.redstone().set_panic(&true);
    g.aqua().set_panic(&true);
    assert_eq!(
        g.strategy().try_divest_otc(&mm, &(1_000 * ONE), &(PRICE_ONE * SCALAR_12)),
        Err(Ok(StrategyError::ExternalError))
    );
}

/// The shipped band is `0`: the only price either fill accepts is the gated rate itself, which
/// is exactly the pre-band behavior. One raw unit either side is refused.
#[test]
fn otc_zero_deviation_admits_only_the_rate() {
    let f = setup(cfg_aqua7());
    let o = otc_setup(&f, 10_500_000); // 1.05
    assert_eq!(f.strategy().get_params().otc_max_deviation_bps, 0);
    let r = rate(&f);
    mint_rwa_to_strategy(&f, 1_000 * ONE);
    for p in [r - 1, r + 1] {
        assert_eq!(f.strategy().try_invest_otc(&o.mm, &(1_000 * ONE), &p), Err(Ok(StrategyError::InvalidArgument)));
        assert_eq!(f.strategy().try_divest_otc(&o.mm, &(1_000 * ONE), &p), Err(Ok(StrategyError::InvalidArgument)));
    }
    assert_eq!(f.strategy().invest_otc(&o.mm, &(1_000 * ONE), &r), 9_523_809_523);
    assert_eq!(f.strategy().divest_otc(&o.mm, &(1_000 * ONE), &r), 9_523_809_523);
}

/// The RFQ case: the counterparty quotes, the keeper passes the agreed price, and the fill
/// sizes the RWA leg at that price. Both directions of both fills work inside the band: a
/// buy above or below the rate, a sell below or above it. Expected legs are
/// `floor(usdc * 10^7 * SCALAR_12 / price)` at rate 1.05.
#[test]
fn otc_fills_at_the_agreed_price_anywhere_inside_the_band() {
    let f = setup(cfg_aqua7());
    let o = otc_setup(&f, 10_500_000); // 1.05
    set_otc_max_deviation(&f, MAX_OTC_DEVIATION_BPS);
    let r = rate(&f);
    mint_rwa_to_strategy(&f, 20_000 * ONE);
    // buy 30 bps over the rate: fewer RWA per USDC than at the rate (9_523_809_523)
    assert_eq!(f.strategy().invest_otc(&o.mm, &(1_000 * ONE), &off(r, 30)), 9_495_323_553);
    // buy 20 bps under the rate: the counterparty's quote is the better deal, so take it
    assert_eq!(f.strategy().invest_otc(&o.mm, &(1_000 * ONE), &off(r, -20)), 9_542_895_314);
    // sell 15 bps under the rate: the counterparty's redemption cost, passed through
    assert_eq!(f.strategy().divest_otc(&o.mm, &(1_000 * ONE), &off(r, -15)), 9_538_116_698);
    // sell 20 bps over the rate
    assert_eq!(f.strategy().divest_otc(&o.mm, &(1_000 * ONE), &off(r, 20)), 9_504_799_923);
}

/// At the cap itself (500 bps) the edges are 0.9975 and 1.1025 at rate 1.05, inclusive, and a
/// raise past the cap is refused at proposal: the timelock can never widen beyond it.
#[test]
fn otc_band_at_the_cap_admits_its_edges_and_nothing_past_them() {
    let f = setup(cfg_aqua7());
    let o = otc_setup(&f, 10_500_000); // 1.05
    assert_eq!(MAX_OTC_DEVIATION_BPS, 500);
    set_otc_max_deviation(&f, MAX_OTC_DEVIATION_BPS);
    let r = rate(&f);
    let (lo, hi) = (off(r, -500), off(r, 500));
    assert_eq!((lo, hi), (9_975_000 * SCALAR_12, 11_025_000 * SCALAR_12));
    mint_rwa_to_strategy(&f, 50_000 * ONE);
    for p in [lo - 1, hi + 1] {
        assert_eq!(f.strategy().try_invest_otc(&o.mm, &(1_000 * ONE), &p), Err(Ok(StrategyError::InvalidArgument)));
        assert_eq!(f.strategy().try_divest_otc(&o.mm, &(1_000 * ONE), &p), Err(Ok(StrategyError::InvalidArgument)));
    }
    for p in [lo, hi] {
        assert!(f.strategy().invest_otc(&o.mm, &(1_000 * ONE), &p) > 0);
        assert!(f.strategy().divest_otc(&o.mm, &(1_000 * ONE), &p) > 0);
    }
    let p = f.strategy().get_params();
    assert_eq!(
        f.strategy().try_propose_params(&p.max_spread_bps, &p.slippage_bps, &p.max_swap, &(MAX_OTC_DEVIATION_BPS + 1)),
        Err(Ok(StrategyError::InvalidArgument))
    );
}

/// Both edges are inclusive; one raw unit past either is `InvalidArgument`, on both fills.
#[test]
fn otc_band_edges_are_inclusive_on_both_fills() {
    let f = setup(cfg_aqua7());
    let o = otc_setup(&f, 10_500_000); // 1.05
    set_otc_max_deviation(&f, 50);
    let r = rate(&f);
    let (lo, hi) = (off(r, -50), off(r, 50));
    assert_eq!((lo, hi), (10_447_500 * SCALAR_12, 10_552_500 * SCALAR_12));
    mint_rwa_to_strategy(&f, 50_000 * ONE);
    for p in [lo - 1, hi + 1] {
        assert_eq!(f.strategy().try_invest_otc(&o.mm, &(1_000 * ONE), &p), Err(Ok(StrategyError::InvalidArgument)));
        assert_eq!(f.strategy().try_divest_otc(&o.mm, &(1_000 * ONE), &p), Err(Ok(StrategyError::InvalidArgument)));
    }
    for p in [lo, hi] {
        assert!(f.strategy().invest_otc(&o.mm, &(1_000 * ONE), &p) > 0);
        assert!(f.strategy().divest_otc(&o.mm, &(1_000 * ONE), &p) > 0);
    }
}

#[test]
fn otc_non_positive_price_is_refused() {
    let f = setup(cfg_aqua7());
    let o = otc_setup(&f, PRICE_ONE);
    mint_rwa_to_strategy(&f, 1_000 * ONE);
    for p in [0, -1] {
        assert_eq!(f.strategy().try_invest_otc(&o.mm, &(1_000 * ONE), &p), Err(Ok(StrategyError::OnlyPositiveAmountAllowed)));
        assert_eq!(f.strategy().try_divest_otc(&o.mm, &(1_000 * ONE), &p), Err(Ok(StrategyError::OnlyPositiveAmountAllowed)));
    }
}

/// At the band's edge the fill pays (`invest_otc`) or receives (`divest_otc`) exactly the
/// price passed, and the event carries that price next to the gated rate, so a monitor reads
/// how far from NAV every fill went. `hi = 10_552_500 * SCALAR_12`, `lo = 10_447_500 *
/// SCALAR_12`. Two independent fixtures: the `divest_otc` leg at `lo` needs more held RWA
/// (9_571_667_863) than the matching `invest_otc` leaves behind (9_476_427_386), so it is
/// minted directly rather than chained off the first call.
#[test]
fn otc_fill_at_the_band_edge_moves_exactly_that_price_and_emits_it() {
    let f = setup(cfg_aqua7());
    let o = otc_setup(&f, 10_500_000); // 1.05
    set_otc_max_deviation(&f, 50);
    let r = rate(&f);

    let mm_rwa_before = f.rwa_token().balance(&o.mm);
    let mm_usdc_before = TokenClient::new(&f.env, &f.usdc).balance(&o.mm);
    let got = f.strategy().invest_otc(&o.mm, &(1_000 * ONE), &off(r, 50));
    // Sampled here: `events().all()` reports only the last top-level invocation, and the
    // balance reads below are each one.
    assert_eq!(f.env.events().all().events().len(), 2, "USDC SAC transfer + OtcInvestEvent");
    assert_event_at(
        &f,
        1,
        &OtcInvestEvent {
            strategy_name: String::from_str(&f.env, STRATEGY_NAME),
            counterparty: o.mm.clone(),
            usdc_in: 1_000 * ONE,
            rwa_out: got,
            price: off(r, 50),
            rate: r,
        },
    );
    assert_eq!(got, 9_476_427_386);
    assert_eq!(f.rwa_token().balance(&o.mm), mm_rwa_before - got, "mm's RWA dropped by exactly the leg");
    assert_eq!(TokenClient::new(&f.env, &f.usdc).balance(&o.mm), mm_usdc_before + 1_000 * ONE, "mm's USDC rose by usdc_amount");

    let f2 = setup(cfg_aqua7());
    let o2 = otc_setup(&f2, 10_500_000); // 1.05
    set_otc_max_deviation(&f2, 50);
    let r2 = rate(&f2);
    mint_rwa_to_strategy(&f2, 9_571_667_863);
    let mm_rwa_before2 = f2.rwa_token().balance(&o2.mm);
    let mm_usdc_before2 = TokenClient::new(&f2.env, &f2.usdc).balance(&o2.mm);
    let paid = f2.strategy().divest_otc(&o2.mm, &(1_000 * ONE), &off(r2, -50));
    assert_eq!(f2.env.events().all().events().len(), 2, "USDC SAC transfer + OtcDivestEvent");
    assert_event_at(
        &f2,
        1,
        &OtcDivestEvent {
            strategy_name: String::from_str(&f2.env, STRATEGY_NAME),
            counterparty: o2.mm.clone(),
            rwa_in: paid,
            usdc_out: 1_000 * ONE,
            price: off(r2, -50),
            rate: r2,
        },
    );
    assert_eq!(paid, 9_571_667_863);
    assert_eq!(f2.rwa_token().balance(&o2.mm), mm_rwa_before2 + paid, "mm's RWA rose by exactly the leg");
    assert_eq!(TokenClient::new(&f2.env, &f2.usdc).balance(&o2.mm), mm_usdc_before2 - 1_000 * ONE, "mm's USDC dropped by usdc_amount");
}

/// The worst case the band allows: a full invest-then-divest round trip on the same USDC
/// value, bought at the top edge and sold at the bottom edge, costs the strategy under
/// `2 * MAX_OTC_DEVIATION_BPS` of the principal, with one raw unit of RWA dust left over (the
/// divest leg's floor leaves the odd raw unit with the strategy). No worse than
/// the stored premium this band replaced, at its cap: identical whenever `rate * (10_000 ± cap)`
/// divides by 10_000 (every fixture rate), and one rate unit tighter on the buy edge otherwise,
/// because the high edge floors where the old `rate_paid` ceiled.
#[test]
fn otc_round_trip_at_the_band_edges_costs_under_twice_the_cap() {
    let f = setup(cfg_aqua7());
    let o = otc_setup(&f, 10_500_000); // 1.05
    set_otc_max_deviation(&f, MAX_OTC_DEVIATION_BPS);
    let r = rate(&f);
    let cap = MAX_OTC_DEVIATION_BPS as i128;

    let q1 = f.strategy().invest_otc(&o.mm, &(500_000 * ONE), &off(r, cap));
    assert_eq!(q1, 4_535_147_392_290);

    let lo = math::apply_bps_ceil(r, -cap).unwrap();
    assert_eq!(lo, off(r, -cap));
    let x = math::value_of(&f.env, q1, lo, f.cfg.rwa_decimals).unwrap();
    assert_eq!(x, 4_523_809_523_809);

    let idle_before = idle(&f);
    let q2 = f.strategy().divest_otc(&o.mm, &x, &lo);
    assert_eq!(q2, 4_535_147_392_289, "one raw unit of dust stays with the strategy");

    assert_eq!(idle(&f), idle_before + x);
    assert_eq!(idle(&f), 1_000_000 * ONE - 500_000 * ONE + x);
    assert_eq!(rwa_held(&f), 1);

    let usdc_lost = 500_000 * ONE - x;
    assert_eq!(usdc_lost, 476_190_476_191);
    // 2 × 500 bps of the 500_000 * ONE principal.
    let cap_bound = 50_000 * ONE;
    assert!(usdc_lost < cap_bound, "round-trip cost {} exceeds the 2x band bound {}", usdc_lost, cap_bound);
}

/// The band reaches the fills through the real admin path: proposed, refused at the fill while
/// it waits, then used once `execute_params` applies it after `TIMELOCK_SECONDS`; narrowing is
/// immediate and binds the next fill.
#[test]
fn otc_band_widened_through_propose_and_execute_then_used() {
    let f = setup(cfg_aqua7());
    let o = otc_setup(&f, 10_500_000); // 1.05
    mint_rwa_to_strategy(&f, 1_000 * ONE);
    let p = f.strategy().get_params();
    f.strategy().propose_params(&p.max_spread_bps, &p.slippage_bps, &p.max_swap, &30);
    let r = rate(&f);
    assert_eq!(
        f.strategy().try_invest_otc(&o.mm, &(1_000 * ONE), &off(r, 30)),
        Err(Ok(StrategyError::InvalidArgument)),
        "a proposed band is not a band yet"
    );
    advance(&f, TIMELOCK_SECONDS, 10_500_000);
    f.strategy().execute_params();
    assert_eq!(f.strategy().get_params().otc_max_deviation_bps, 30);
    let r = rate(&f);
    assert_eq!(r, 10_500_000 * SCALAR_12);
    assert_eq!(f.strategy().invest_otc(&o.mm, &(1_000 * ONE), &off(r, 30)), 9_495_323_553);
    assert_eq!(
        f.strategy().try_divest_otc(&o.mm, &(1_000 * ONE), &(off(r, -30) - 1)),
        Err(Ok(StrategyError::InvalidArgument))
    );
    assert!(f.strategy().divest_otc(&o.mm, &(1_000 * ONE), &off(r, -30)) > 0);
    f.strategy().set_otc_max_deviation_bps(&10);
    assert_eq!(
        f.strategy().try_invest_otc(&o.mm, &(1_000 * ONE), &off(r, 30)),
        Err(Ok(StrategyError::InvalidArgument))
    );
}

/// `Frozen` at the anchor with a band: the refill still prices off the anchor, edges included,
/// whatever the oracles now say.
#[test]
fn divest_otc_in_frozen_honors_the_band_around_the_anchor() {
    let f = setup(cfg_aqua7());
    let o = otc_setup(&f, PRICE_ONE);
    f.strategy().invest_otc(&o.mm, &(100_000 * ONE), &rate(&f));
    set_otc_max_deviation(&f, 50);
    f.strategy().freeze_price(&f.keeper, &true);
    advance(&f, crate::constants::ANCHOR_WINDOW_SECONDS, 9_000_000); // oracles at 0.90
    let anchor = PRICE_ONE * SCALAR_12;
    assert_eq!(rate(&f), anchor);
    let lo = off(anchor, -50);
    assert_eq!(
        f.strategy().try_divest_otc(&o.mm, &(1_000 * ONE), &(lo - 1)),
        Err(Ok(StrategyError::InvalidArgument))
    );
    // 1_000 USDC at 0.995 per RWA: floor(1_000 / 0.995 * 10^7) raw units
    assert_eq!(f.strategy().divest_otc(&o.mm, &(1_000 * ONE), &lo), 10_050_251_256);
}

/// An 18-decimal RWA (deJTRSY's shape, `cfg_sushi18`): the price is the same number as for a
/// 7-decimal RWA, because the rate is USDC stroops per *whole* RWA times `SCALAR_12`; only the
/// RWA leg scales with `10^rwa_decimals`. At 1.25: 1M USDC buys exactly 800_000 whole RWA
/// (`800_000 * 10^18` raw); with a 50 bps band the edges are 1.24375 and 1.25625, a sell at
/// the low edge pays `floor(500_000 / 1.24375 * 10^18)` raw and a buy at the high edge pulls
/// `floor(100_000 / 1.25625 * 10^18)` raw. All of it through I256, no overflow.
#[test]
fn otc_fills_on_an_18_decimal_rwa_at_the_same_price_units() {
    let f = setup(cfg_sushi18());
    let o = otc_setup(&f, 12_500_000); // 1.25
    let r = rate(&f);
    assert_eq!(r, 12_500_000 * SCALAR_12, "same price unit as a 7-dp RWA");

    let got = f.strategy().invest_otc(&o.mm, &(1_000_000 * ONE), &r);
    assert_eq!(got, 800_000 * ONE_18);
    assert_eq!(rwa_held(&f), 800_000 * ONE_18);
    assert_eq!(idle(&f), 0);
    assert_near(f.strategy().balance(&o.vault), 1_000_000 * ONE, 10);

    set_otc_max_deviation(&f, 50);
    let (lo, hi) = (off(r, -50), off(r, 50));
    assert_eq!((lo, hi), (12_437_500 * SCALAR_12, 12_562_500 * SCALAR_12));
    assert_eq!(
        f.strategy().try_divest_otc(&o.mm, &(500_000 * ONE), &(lo - 1)),
        Err(Ok(StrategyError::InvalidArgument))
    );
    let mm_rwa_before = f.rwa_token().balance(&o.mm);
    let paid = f.strategy().divest_otc(&o.mm, &(500_000 * ONE), &lo);
    assert_eq!(paid, 402_010_050_251_256_281_407_035);
    assert_eq!(f.rwa_token().balance(&o.mm), mm_rwa_before + paid);
    assert_eq!(rwa_held(&f), 800_000 * ONE_18 - paid);
    assert_eq!(idle(&f), 500_000 * ONE);

    assert_eq!(
        f.strategy().try_invest_otc(&o.mm, &(100_000 * ONE), &(hi + 1)),
        Err(Ok(StrategyError::InvalidArgument))
    );
    assert_eq!(f.strategy().invest_otc(&o.mm, &(100_000 * ONE), &hi), 79_601_990_049_751_243_781_094);
}

#[test]
fn buys_refuse_while_the_oracle_reads_below_the_clamped_rate_and_sells_still_run() {
    // Live, but the oracle fell faster than the clamp lets NAV follow: `rate` (L * 0.99) sits
    // above what the oracle says (0.95). Buying at `rate` would overpay, so every buy refuses;
    // the refill side (`divest_otc`) still runs, at `rate`, which favours the strategy.
    let f = setup(cfg_aqua7());
    let o = otc_setup(&f, PRICE_ONE); // anchor 1.00
    f.strategy().invest_otc(&o.mm, &(10_000 * ONE), &rate(&f));
    let t = f.env.ledger().timestamp();
    f.sep40().set_price(&(9_500_000 * 10_000_000), &t);
    f.redstone().set_price(&((9_500_000 * 10) as u128), &(t * 1_000));
    let s = f.strategy().get_pricing_state();
    assert_eq!((s.mode, s.rate, s.raw_rate), (crate::PricingMode::Live, 9_900_000 * SCALAR_12, 9_500_000 * SCALAR_12));
    assert_eq!(
        f.strategy().try_invest_otc(&o.mm, &(1_000 * ONE), &rate(&f)),
        Err(Ok(StrategyError::ExternalError))
    );
    assert_eq!(f.strategy().try_invest(&(1_000 * ONE)), Err(Ok(StrategyError::ExternalError)));
    assert!(f.strategy().divest_otc(&o.mm, &(990 * ONE), &rate(&f)) > 0);
}
