#![cfg(test)]
extern crate std;

use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::Address;

use crate::constants::TIMELOCK_SECONDS;
use crate::testutils::*;
use crate::StrategyError;

#[test]
fn admin_transfer_is_two_step() {
    let f = setup(cfg_aqua7());
    let new_admin = Address::generate(&f.env);
    assert_eq!(f.strategy().try_accept_admin(), Err(Ok(StrategyError::NotAuthorized)), "nothing pending");
    f.strategy().transfer_admin(&new_admin);
    assert_eq!(f.strategy().get_params().pending_admin, Some(new_admin.clone()));
    assert_eq!(f.strategy().get_params().admin, f.admin, "unchanged until accepted");
    f.strategy().accept_admin();
    // Sampled here: `events().all()` reports only the last top-level invocation, and the
    // `get_params` call below is one.
    let emitted = f.env.events().all().events().len();
    let p = f.strategy().get_params();
    assert_eq!(p.admin, new_admin);
    assert_eq!(p.pending_admin, None);
    assert_eq!(emitted, 1, "AdminTransferredEvent");
}

#[test]
fn set_keeper_is_admin_only_and_rotates() {
    let f = setup(cfg_aqua7());
    let bot2 = Address::generate(&f.env);
    f.strategy().set_keeper(&bot2);
    assert_eq!(f.strategy().get_params().keeper, bot2);
    // the old keeper can no longer harvest (address compare on the dual check)
    assert_eq!(f.strategy().try_harvest(&f.keeper, &None), Err(Ok(StrategyError::NotAuthorized)));
}

#[test]
fn set_caps_and_pricing_params_validate_and_emit_per_field() {
    let f = setup(cfg_aqua7());
    assert_eq!(f.strategy().try_set_caps(&0, &ONE), Err(Ok(StrategyError::InvalidArgument)));
    assert_eq!(f.strategy().try_set_caps(&ONE, &0), Err(Ok(StrategyError::InvalidArgument)));
    // `max_swap` 10_000 -> 20_000 is a raise, so the immediate setter refuses it and
    // the 72 h proposal carries it. The other three knobs are proposed at their live values.
    assert_eq!(
        f.strategy().try_set_caps(&(20_000 * ONE), &(2_000_000 * ONE)),
        Err(Ok(StrategyError::NotAuthorized)),
        "max_swap raised"
    );
    f.strategy().propose_params(&100, &100, &(20_000 * ONE), &0);
    advance(&f, TIMELOCK_SECONDS, PRICE_ONE);
    f.strategy().execute_params();
    assert_eq!(f.env.events().all().events().len(), 1, "only max_swap changed");
    // `max_invested` is never timelocked, so the original call now goes through as written.
    f.strategy().set_caps(&(20_000 * ONE), &(2_000_000 * ONE));
    assert_eq!(f.env.events().all().events().len(), 2);
    let p = f.strategy().get_params();
    assert_eq!((p.max_swap, p.max_invested), (20_000 * ONE, 2_000_000 * ONE));

    assert_eq!(f.strategy().try_set_pricing_params(&9, &100), Err(Ok(StrategyError::InvalidArgument)));
    assert_eq!(f.strategy().try_set_pricing_params(&501, &100), Err(Ok(StrategyError::InvalidArgument)), "cap is 500");
    assert_eq!(f.strategy().try_set_pricing_params(&100, &501), Err(Ok(StrategyError::InvalidArgument)));
    // 100 -> 250 and 100 -> 150 are raises: refused here, proposed and executed instead.
    assert_eq!(
        f.strategy().try_set_pricing_params(&250, &150),
        Err(Ok(StrategyError::NotAuthorized)),
        "max_spread_bps and slippage_bps raised"
    );
    f.strategy().propose_params(&250, &150, &(20_000 * ONE), &0);
    advance(&f, TIMELOCK_SECONDS, PRICE_ONE);
    f.strategy().execute_params();
    assert_eq!(f.env.events().all().events().len(), 2, "max_spread_bps and slippage_bps changed");
    // Lowering is immediate, one event per field.
    f.strategy().set_pricing_params(&25, &50);
    assert_eq!(f.env.events().all().events().len(), 2);
    let p = f.strategy().get_params();
    assert_eq!((p.max_spread_bps, p.slippage_bps), (25, 50));
}

#[test]
fn freeze_price_keeper_freeze_lapses_admin_freeze_holds_only_admin_unfreezes() {
    use crate::constants::KEEPER_FREEZE_SECONDS;
    let f = setup(cfg_aqua7());
    let stranger = soroban_sdk::Address::generate(&f.env);
    assert_eq!(f.strategy().try_freeze_price(&stranger, &true), Err(Ok(StrategyError::NotAuthorized)));
    assert_eq!(f.strategy().get_params().frozen_until, 0);

    // Keeper: 24 h from now, then it lapses on its own.
    f.strategy().freeze_price(&f.keeper, &true);
    assert_eq!(f.env.events().all().events().len(), 1, "PriceFrozenEvent");
    assert_eq!(f.strategy().get_params().frozen_until, START_TS + KEEPER_FREEZE_SECONDS);
    assert_eq!(f.strategy().get_pricing_state().mode, crate::PricingMode::Frozen);
    assert_eq!(
        f.strategy().try_freeze_price(&f.keeper, &false),
        Err(Ok(StrategyError::NotAuthorized)),
        "a compromised keeper cannot lift a freeze the admin needs"
    );
    assert_eq!(f.strategy().try_freeze_price(&stranger, &false), Err(Ok(StrategyError::NotAuthorized)));
    advance(&f, KEEPER_FREEZE_SECONDS - 1, PRICE_ONE);
    assert_eq!(f.strategy().get_pricing_state().mode, crate::PricingMode::Frozen);
    advance(&f, 1, PRICE_ONE);
    assert_eq!(f.strategy().get_pricing_state().mode, crate::PricingMode::Live, "lapsed");

    // Spent: the keeper cannot freeze again after its freeze lapses.
    assert!(f.strategy().get_params().keeper_freeze_spent);
    assert_eq!(f.strategy().try_freeze_price(&f.keeper, &true), Err(Ok(StrategyError::NotAuthorized)));

    // Admin: indefinite, and a keeper freeze (re-armed by the admin) never shortens it.
    f.strategy().freeze_price(&f.admin, &false);
    assert!(!f.strategy().get_params().keeper_freeze_spent, "an admin unfreeze re-arms it");
    f.strategy().freeze_price(&f.admin, &true);
    assert_eq!(f.strategy().get_params().frozen_until, u64::MAX);
    f.strategy().freeze_price(&f.keeper, &true);
    assert_eq!(f.strategy().get_params().frozen_until, u64::MAX);
    advance(&f, 10 * KEEPER_FREEZE_SECONDS, PRICE_ONE);
    assert_eq!(f.strategy().get_pricing_state().mode, crate::PricingMode::Frozen);
    f.strategy().freeze_price(&f.admin, &false);
    assert_eq!(f.strategy().get_params().frozen_until, 0);
    assert_eq!(f.strategy().get_pricing_state().mode, crate::PricingMode::Live);
}

/// Finding: a keeper re-freezing every 23 h, or right after an admin unfreeze, would hold the
/// rate forever. One freeze per arming bounds a compromised keeper at `KEEPER_FREEZE_SECONDS`.
#[test]
fn keeper_freeze_is_one_shot_until_the_admin_rearms_it() {
    use crate::constants::KEEPER_FREEZE_SECONDS;
    let f = setup(cfg_aqua7());
    f.strategy().freeze_price(&f.keeper, &true);
    let until = START_TS + KEEPER_FREEZE_SECONDS;

    // Refreshing inside the window is refused and does not extend the freeze.
    advance(&f, KEEPER_FREEZE_SECONDS - 3_600, PRICE_ONE);
    assert_eq!(f.strategy().try_freeze_price(&f.keeper, &true), Err(Ok(StrategyError::NotAuthorized)));
    assert_eq!(f.strategy().get_params().frozen_until, until);
    advance(&f, 3_600, PRICE_ONE);
    assert_eq!(f.strategy().get_pricing_state().mode, crate::PricingMode::Live, "lapsed on its own");

    // An admin unfreeze re-arms exactly one more freeze.
    f.strategy().freeze_price(&f.admin, &false);
    f.strategy().freeze_price(&f.keeper, &true);
    f.strategy().freeze_price(&f.admin, &false);
    assert_eq!(f.strategy().get_params().frozen_until, 0);
    f.strategy().freeze_price(&f.keeper, &true);
    assert_eq!(f.strategy().try_freeze_price(&f.keeper, &true), Err(Ok(StrategyError::NotAuthorized)));

    // A new keeper starts armed; the old key is no longer the keeper at all.
    let old = f.keeper.clone();
    let bot2 = Address::generate(&f.env);
    f.strategy().set_keeper(&bot2);
    assert!(!f.strategy().get_params().keeper_freeze_spent);
    assert_eq!(f.strategy().try_freeze_price(&old, &true), Err(Ok(StrategyError::NotAuthorized)));
    advance(&f, KEEPER_FREEZE_SECONDS, PRICE_ONE);
    f.strategy().freeze_price(&bot2, &true);
    assert!(f.strategy().get_params().keeper_freeze_spent);
}

#[test]
fn keeper_cannot_call_admin_methods_and_admin_cannot_call_keeper_methods() {
    // Under mock_all_auths the signer is whoever the contract asks for, so the role split is
    // proven by the real-auth integration test `roles_are_enforced_with_real_auth` (Task 18).
    // Here we prove the address-compare gates: harvest and pause.
    let f = setup(cfg_aqua7());
    assert_eq!(f.strategy().try_harvest(&f.admin, &None), Err(Ok(StrategyError::NotAuthorized)));
    let stranger = Address::generate(&f.env);
    assert_eq!(f.strategy().try_pause_deposits(&stranger, &true), Err(Ok(StrategyError::NotAuthorized)));
}
