#![cfg(test)]
extern crate std;

use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::Address;

use crate::constants::TIMELOCK_SECONDS;
use crate::events::{ParamsProposalCancelledEvent, ParamsProposedEvent, ParamsUpdatedEvent};
use crate::storage::{OracleKind, OracleSource, PendingParams};
use crate::testutils::*;
use crate::StrategyError;

/// A SEP-40 mirror at `price` (USDC per whole RWA, 7 dp), stamped now.
fn mirror(f: &Fx, price: i128) -> (Address, OracleSource) {
    let feed = f.env.register(MockSep40, (SEP40_DECIMALS, price * 10_000_000, now(f)));
    let src = OracleSource { kind: OracleKind::Sep40(feed.clone(), f.rwa.clone()), ttl_seconds: 900, decimals: SEP40_DECIMALS };
    (feed, src)
}

/// Re-stamps a mirror after the clock moved, keeping its price.
fn restamp(f: &Fx, feed: &Address, price: i128) {
    MockSep40Client::new(&f.env, feed).set_price(&(price * 10_000_000), &now(f));
}

#[test]
fn propose_oracles_validates_like_the_constructor_and_changes_nothing_yet() {
    let f = setup(cfg_aqua7());
    let red = f.oracles.get(1).unwrap();
    let at = f.strategy().propose_oracles(&soroban_sdk::vec![&f.env, red.clone()]);
    assert_eq!(at, START_TS + TIMELOCK_SECONDS);
    assert_eq!(f.env.events().all().events().len(), 1, "OraclesProposedEvent");
    assert_eq!(f.strategy().get_pending_oracles().unwrap().executable_at, at);
    assert_eq!(f.strategy().get_params().oracles.len(), 2, "unchanged until executed");
    assert_eq!(f.strategy().get_pricing_state().live_oracles, 2);
    // invalid sets are refused at proposal time
    let empty: soroban_sdk::Vec<OracleSource> = soroban_sdk::vec![&f.env];
    assert_eq!(f.strategy().try_propose_oracles(&empty), Err(Ok(StrategyError::InvalidArgument)));
    let mut bad = red.clone();
    bad.ttl_seconds = 86_401;
    assert_eq!(f.strategy().try_propose_oracles(&soroban_sdk::vec![&f.env, bad]), Err(Ok(StrategyError::InvalidArgument)));
    let mut bad = red.clone();
    bad.decimals = 14; // live adapter says 8
    assert_eq!(f.strategy().try_propose_oracles(&soroban_sdk::vec![&f.env, bad]), Err(Ok(StrategyError::InvalidArgument)));
    let dead = OracleSource { kind: OracleKind::RedStone(Address::generate(&f.env)), ttl_seconds: 900, decimals: 8 };
    assert_eq!(f.strategy().try_propose_oracles(&soroban_sdk::vec![&f.env, dead]), Err(Ok(StrategyError::InvalidArgument)));
}

#[test]
fn execute_oracles_after_the_wait_repoints_pricing() {
    let f = setup(cfg_aqua7());
    let red = f.oracles.get(1).unwrap();
    f.strategy().propose_oracles(&soroban_sdk::vec![&f.env, red]);
    assert_eq!(f.strategy().try_execute_oracles(), Err(Ok(StrategyError::NotAuthorized)), "too early");
    advance(&f, TIMELOCK_SECONDS - 1, PRICE_ONE);
    assert_eq!(f.strategy().try_execute_oracles(), Err(Ok(StrategyError::NotAuthorized)), "one second early");
    advance(&f, 1, PRICE_ONE);
    assert_eq!(f.strategy().execute_oracles(), 1);
    assert_eq!(f.env.events().all().events().len(), 1, "OraclesSetEvent");
    assert_eq!(f.strategy().get_params().oracles.len(), 1);
    assert_eq!(f.strategy().get_pending_oracles(), None);
    f.sep40().set_panic(&true); // the old source no longer matters
    let s = f.strategy().get_pricing_state();
    assert_eq!(s.live_oracles, 1);
    assert_eq!(s.mode, crate::PricingMode::Live);
    // nothing pending: execute and cancel are refused
    assert_eq!(f.strategy().try_execute_oracles(), Err(Ok(StrategyError::InvalidArgument)));
    assert_eq!(f.strategy().try_cancel_oracles(), Err(Ok(StrategyError::InvalidArgument)));
}

#[test]
fn execute_oracles_needs_one_live_source_and_nothing_else() {
    let f = setup(cfg_aqua7()); // max_spread_bps 100
    f.strategy().harvest(&f.keeper, &None); // anchors L = 1.00
    // Every proposed source dead at execution: refused, old set and proposal stay.
    let (feed, src) = mirror(&f, PRICE_ONE);
    f.strategy().propose_oracles(&soroban_sdk::vec![&f.env, src]);
    advance(&f, TIMELOCK_SECONDS, PRICE_ONE);
    MockSep40Client::new(&f.env, &feed).set_panic(&true);
    assert_eq!(f.strategy().try_execute_oracles(), Err(Ok(StrategyError::ExternalError)));
    assert_eq!(f.strategy().get_params().oracles.len(), 2);
    assert!(f.strategy().get_pending_oracles().is_some(), "the admin can still cancel or wait");
    // Live again: accepted.
    MockSep40Client::new(&f.env, &feed).set_panic(&false);
    restamp(&f, &feed, PRICE_ONE);
    assert_eq!(f.strategy().execute_oracles(), 1);
}

#[test]
fn a_correct_feed_replaces_a_wrong_one_however_far_the_anchor_walked() {
    // A wrong low oracle walks the anchor down 1% per window. Nine windows later the anchor
    // is ~8.6% under the truth; the correct feed must still be accepted, and the rate then
    // climbs back one spread step per window.
    let f = setup(Cfg { oracles: OracleSet::RedStoneOnly, ..cfg_aqua7() });
    f.strategy().harvest(&f.keeper, &None); // L = 1.00
    for _ in 0..9 {
        advance(&f, crate::constants::ANCHOR_WINDOW_SECONDS, 5_000_000); // the feed says 0.50
        f.strategy().harvest(&f.keeper, &None);
    }
    let walked = f.env.as_contract(&f.strategy_id, || crate::storage::get_last_valid_price(&f.env)).unwrap();
    assert!(walked < 9_150_000 * crate::constants::SCALAR_12, "anchor walked to {}", walked);
    let (feed_ok, src_ok) = mirror(&f, PRICE_ONE);
    f.strategy().propose_oracles(&soroban_sdk::vec![&f.env, src_ok]);
    f.strategy().freeze_price(&f.admin, &true); // hold it while the 72 h runs
    advance(&f, TIMELOCK_SECONDS, 5_000_000);
    restamp(&f, &feed_ok, PRICE_ONE);
    assert_eq!(f.strategy().execute_oracles(), 1);
    f.strategy().freeze_price(&f.admin, &false);
    let s = f.strategy().get_pricing_state();
    assert_eq!(s.raw_rate, PRICE_ONE * crate::constants::SCALAR_12);
    assert_eq!(s.rate, walked * 101 / 100, "one spread step above the walked anchor");
}

#[test]
fn a_new_proposal_replaces_the_old_and_cancel_clears() {
    let f = setup(cfg_aqua7());
    let sep = f.oracles.get(0).unwrap();
    let red = f.oracles.get(1).unwrap();
    f.strategy().propose_oracles(&soroban_sdk::vec![&f.env, sep.clone(), red.clone()]);
    f.strategy().propose_oracles(&soroban_sdk::vec![&f.env, red.clone()]);
    assert_eq!(f.strategy().get_pending_oracles().unwrap().oracles.len(), 1, "the later proposal wins");
    f.strategy().cancel_oracles();
    assert_eq!(f.env.events().all().events().len(), 1, "OraclesProposalCancelledEvent");
    assert_eq!(f.strategy().get_pending_oracles(), None);
    advance(&f, TIMELOCK_SECONDS, PRICE_ONE);
    assert_eq!(f.strategy().try_execute_oracles(), Err(Ok(StrategyError::InvalidArgument)));
    assert_eq!(f.strategy().get_params().oracles.len(), 2);
}

#[test]
fn counterparty_proposal_waits_execute_allows_and_remove_is_immediate() {
    let f = setup(cfg_aqua7());
    let cp = Address::generate(&f.env);
    assert_eq!(f.strategy().try_execute_otc_counterparty(&cp), Err(Ok(StrategyError::InvalidArgument)), "nothing proposed");
    let at = f.strategy().propose_otc_counterparty(&cp);
    assert_eq!(at, START_TS + TIMELOCK_SECONDS);
    assert_eq!(f.env.events().all().events().len(), 1, "OtcCounterpartyProposedEvent");
    assert_eq!(f.strategy().get_pending_otc_counterparty(&cp), Some(at));
    assert!(!f.strategy().is_otc_counterparty(&cp), "proposed is not allowed");
    assert_eq!(f.strategy().try_execute_otc_counterparty(&cp), Err(Ok(StrategyError::NotAuthorized)), "too early");
    advance(&f, TIMELOCK_SECONDS, PRICE_ONE);
    f.strategy().execute_otc_counterparty(&cp);
    assert_eq!(f.env.events().all().events().len(), 1, "OtcCounterpartySetEvent");
    assert!(f.strategy().is_otc_counterparty(&cp));
    assert_eq!(f.strategy().get_pending_otc_counterparty(&cp), None);
    // removing never waits, and clears a pending proposal too
    f.strategy().remove_otc_counterparty(&cp);
    assert!(!f.strategy().is_otc_counterparty(&cp));
    f.strategy().propose_otc_counterparty(&cp);
    f.strategy().remove_otc_counterparty(&cp);
    assert_eq!(f.strategy().get_pending_otc_counterparty(&cp), None);
    advance(&f, TIMELOCK_SECONDS, PRICE_ONE);
    assert_eq!(f.strategy().try_execute_otc_counterparty(&cp), Err(Ok(StrategyError::InvalidArgument)));
}

// ===========================================================================
// The four price knobs: one proposal to raise, immediate setters to lower.
// ===========================================================================

// `cfg_aqua7`'s knobs, written out (`src/testutils.rs:454-470`), so every expectation below
// reads as "the current value" versus "+50".
const CFG_SPREAD: u32 = 100; // max_spread_bps
const CFG_SLIPPAGE: u32 = 100; // slippage_bps
const CFG_MAX_SWAP: i128 = 10_000 * ONE;

/// Compares the `i`-th event of the last top-level invocation against the event the caller
/// expected. `Event::to_xdr` is the SDK's own "this contract published exactly this" form, so
/// the topics and every field are checked, not just the count.
fn assert_event_at<E: soroban_sdk::Event>(f: &Fx, i: usize, expected: &E) {
    let all = f.env.events().all();
    assert_eq!(all.events()[i], expected.to_xdr(&f.env, &f.strategy_id), "event #{}", i);
}

/// One `ParamsUpdatedEvent` for `field`, as `execute_params` and the immediate setters emit it.
fn params_updated(f: &Fx, field: &str, old_value: i128, new_value: i128) -> ParamsUpdatedEvent {
    ParamsUpdatedEvent {
        strategy_name: soroban_sdk::String::from_str(&f.env, crate::constants::STRATEGY_NAME),
        field: soroban_sdk::Symbol::new(&f.env, field),
        old_value,
        new_value,
    }
}

#[test]
fn propose_params_validates_and_changes_nothing_yet() {
    let f = setup(cfg_aqua7());
    // Every constructor bound, plus the OTC band cap, refused at proposal time.
    for (spread, slippage, max_swap, band, why) in [
        (9u32, CFG_SLIPPAGE, CFG_MAX_SWAP, 0u32, "spread below MIN_SPREAD_BPS"),
        (501, CFG_SLIPPAGE, CFG_MAX_SWAP, 0, "spread above MAX_SPREAD_BPS"),
        (CFG_SPREAD, 501, CFG_MAX_SWAP, 0, "slippage above MAX_SLIPPAGE_BPS"),
        (CFG_SPREAD, CFG_SLIPPAGE, 0, 0, "max_swap not positive"),
        (CFG_SPREAD, CFG_SLIPPAGE, CFG_MAX_SWAP, 501, "band above MAX_OTC_DEVIATION_BPS"),
    ] {
        assert_eq!(
            f.strategy().try_propose_params(&spread, &slippage, &max_swap, &band),
            Err(Ok(StrategyError::InvalidArgument)),
            "{}",
            why
        );
    }

    let at = f.strategy().propose_params(&(CFG_SPREAD + 50), &(CFG_SLIPPAGE + 50), &(CFG_MAX_SWAP * 2), &50);
    assert_eq!(at, now(&f) + TIMELOCK_SECONDS);
    assert_eq!(f.env.events().all().events().len(), 1, "ParamsProposedEvent");
    assert_event_at(
        &f,
        0,
        &ParamsProposedEvent {
            strategy_name: soroban_sdk::String::from_str(&f.env, crate::constants::STRATEGY_NAME),
            max_spread_bps: CFG_SPREAD + 50,
            slippage_bps: CFG_SLIPPAGE + 50,
            max_swap: CFG_MAX_SWAP * 2,
            otc_max_deviation_bps: 50,
            executable_at: at,
        },
    );
    assert_eq!(
        f.strategy().get_pending_params(),
        Some(PendingParams {
            max_spread_bps: CFG_SPREAD + 50,
            slippage_bps: CFG_SLIPPAGE + 50,
            max_swap: CFG_MAX_SWAP * 2,
            otc_max_deviation_bps: 50,
            executable_at: at,
        })
    );
    let p = f.strategy().get_params();
    assert_eq!(
        (p.max_spread_bps, p.slippage_bps, p.max_swap, p.otc_max_deviation_bps),
        (CFG_SPREAD, CFG_SLIPPAGE, CFG_MAX_SWAP, 0),
        "unchanged until executed"
    );
}

#[test]
fn execute_params_refuses_early_then_applies_and_clears() {
    let f = setup(cfg_aqua7());
    f.strategy().propose_params(&(CFG_SPREAD + 50), &(CFG_SLIPPAGE + 50), &(CFG_MAX_SWAP * 2), &50);
    assert_eq!(f.strategy().try_execute_params(), Err(Ok(StrategyError::NotAuthorized)), "too early");
    advance(&f, TIMELOCK_SECONDS - 1, PRICE_ONE);
    assert_eq!(f.strategy().try_execute_params(), Err(Ok(StrategyError::NotAuthorized)), "one second early");
    advance(&f, 1, PRICE_ONE);

    f.strategy().execute_params();
    // Sampled before the `get_params` below: `events().all()` reports only the last invocation.
    assert_eq!(f.env.events().all().events().len(), 4, "one per changed field");
    assert_event_at(&f, 0, &params_updated(&f, "max_spread_bps", CFG_SPREAD as i128, (CFG_SPREAD + 50) as i128));
    assert_event_at(&f, 1, &params_updated(&f, "slippage_bps", CFG_SLIPPAGE as i128, (CFG_SLIPPAGE + 50) as i128));
    assert_event_at(&f, 2, &params_updated(&f, "max_swap", CFG_MAX_SWAP, CFG_MAX_SWAP * 2));
    assert_event_at(&f, 3, &params_updated(&f, "otc_max_deviation_bps", 0, 50));
    let p = f.strategy().get_params();
    assert_eq!(
        (p.max_spread_bps, p.slippage_bps, p.max_swap, p.otc_max_deviation_bps),
        (CFG_SPREAD + 50, CFG_SLIPPAGE + 50, CFG_MAX_SWAP * 2, 50)
    );
    assert_eq!(f.strategy().get_pending_params(), None);

    // A proposal identical to the live values executes and emits nothing: only changed fields.
    f.strategy().propose_params(&(CFG_SPREAD + 50), &(CFG_SLIPPAGE + 50), &(CFG_MAX_SWAP * 2), &50);
    advance(&f, TIMELOCK_SECONDS, PRICE_ONE);
    f.strategy().execute_params();
    assert_eq!(f.env.events().all().events().len(), 0, "nothing changed, nothing emitted");
}

#[test]
fn execute_and_cancel_params_refuse_without_a_proposal() {
    let f = setup(cfg_aqua7());
    assert_eq!(f.strategy().try_execute_params(), Err(Ok(StrategyError::InvalidArgument)));
    assert_eq!(f.strategy().try_cancel_params(), Err(Ok(StrategyError::InvalidArgument)));
}

#[test]
fn a_new_params_proposal_replaces_the_old_and_cancel_clears() {
    let f = setup(cfg_aqua7());
    f.strategy().propose_params(&200, &200, &(CFG_MAX_SWAP * 3), &10);
    advance(&f, 10, PRICE_ONE); // so "fresh executable_at" is provable, not coincidental
    let at_b = f.strategy().propose_params(&(CFG_SPREAD + 50), &(CFG_SLIPPAGE + 50), &(CFG_MAX_SWAP * 2), &50);
    assert_eq!(
        f.strategy().get_pending_params(),
        Some(PendingParams {
            max_spread_bps: CFG_SPREAD + 50,
            slippage_bps: CFG_SLIPPAGE + 50,
            max_swap: CFG_MAX_SWAP * 2,
            otc_max_deviation_bps: 50,
            executable_at: at_b,
        }),
        "the later proposal wins, with a fresh wait"
    );
    assert_eq!(at_b, now(&f) + TIMELOCK_SECONDS);

    f.strategy().cancel_params();
    assert_eq!(f.env.events().all().events().len(), 1, "ParamsProposalCancelledEvent");
    assert_event_at(
        &f,
        0,
        &ParamsProposalCancelledEvent {
            strategy_name: soroban_sdk::String::from_str(&f.env, crate::constants::STRATEGY_NAME),
        },
    );
    assert_eq!(f.strategy().get_pending_params(), None);
    assert_eq!(f.strategy().try_cancel_params(), Err(Ok(StrategyError::InvalidArgument)));
    advance(&f, TIMELOCK_SECONDS, PRICE_ONE);
    assert_eq!(f.strategy().try_execute_params(), Err(Ok(StrategyError::InvalidArgument)));
    let p = f.strategy().get_params();
    assert_eq!((p.max_spread_bps, p.slippage_bps, p.max_swap, p.otc_max_deviation_bps), (CFG_SPREAD, CFG_SLIPPAGE, CFG_MAX_SWAP, 0));
}

#[test]
fn immediate_setters_only_lower() {
    let f = setup(cfg_aqua7());
    f.strategy().propose_params(&(CFG_SPREAD + 50), &(CFG_SLIPPAGE + 50), &(CFG_MAX_SWAP * 2), &50);
    advance(&f, TIMELOCK_SECONDS, PRICE_ONE);
    f.strategy().execute_params(); // 150 / 150 / 20_000 USDC / 50 bps

    // A raise of either pricing knob is refused, one field at a time.
    assert_eq!(
        f.strategy().try_set_pricing_params(&(CFG_SPREAD + 51), &(CFG_SLIPPAGE + 50)),
        Err(Ok(StrategyError::NotAuthorized)),
        "max_spread_bps raised"
    );
    assert_eq!(
        f.strategy().try_set_pricing_params(&(CFG_SPREAD + 50), &(CFG_SLIPPAGE + 51)),
        Err(Ok(StrategyError::NotAuthorized)),
        "slippage_bps raised"
    );
    // Out of range still reports the range error, checked before the raise rule.
    assert_eq!(
        f.strategy().try_set_pricing_params(&501, &CFG_SLIPPAGE),
        Err(Ok(StrategyError::InvalidArgument)),
        "range before raise"
    );
    // Lowering both is immediate, one event per field (`src/lib.rs` `set_pricing_params`).
    f.strategy().set_pricing_params(&CFG_SPREAD, &CFG_SLIPPAGE);
    assert_eq!(f.env.events().all().events().len(), 2, "spread and slippage");
    assert_event_at(&f, 0, &params_updated(&f, "max_spread_bps", (CFG_SPREAD + 50) as i128, CFG_SPREAD as i128));
    assert_event_at(&f, 1, &params_updated(&f, "slippage_bps", (CFG_SLIPPAGE + 50) as i128, CFG_SLIPPAGE as i128));

    // `max_swap` lowers here; `max_invested` moves freely either way.
    assert_eq!(
        f.strategy().try_set_caps(&(CFG_MAX_SWAP * 2 + 1), &(CFG_MAX_SWAP * 10)),
        Err(Ok(StrategyError::NotAuthorized)),
        "max_swap raised"
    );
    f.strategy().set_caps(&CFG_MAX_SWAP, &(CFG_MAX_SWAP * 10));
    let p = f.strategy().get_params();
    assert_eq!((p.max_swap, p.max_invested), (CFG_MAX_SWAP, CFG_MAX_SWAP * 10));

    // The OTC band: range first, then lower-only, and equal is not a raise.
    assert_eq!(f.strategy().try_set_otc_max_deviation_bps(&501), Err(Ok(StrategyError::InvalidArgument)), "above the cap");
    assert_eq!(f.strategy().try_set_otc_max_deviation_bps(&50), Ok(Ok(())), "equal is not a raise");
    f.strategy().set_otc_max_deviation_bps(&20);
    assert_eq!(f.env.events().all().events().len(), 1, "ParamsUpdatedEvent");
    assert_event_at(&f, 0, &params_updated(&f, "otc_max_deviation_bps", 50, 20));
    assert_eq!(f.strategy().get_params().otc_max_deviation_bps, 20);
    assert_eq!(f.strategy().try_set_otc_max_deviation_bps(&21), Err(Ok(StrategyError::NotAuthorized)), "raised");
}
