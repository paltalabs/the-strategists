#![cfg(test)]
extern crate std;

use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::token::TokenClient;
use soroban_sdk::Address;

use crate::pricing::PricingMode;
use crate::testutils::*;
use crate::StrategyError;

#[test]
fn deposit_accepts_any_amount_into_idle_and_mints_at_par() {
    let f = setup(cfg_aqua7());
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 500_000 * ONE); // 50x max_swap
    let bal = f.strategy().deposit(&(500_000 * ONE), &vault);
    assert_eq!(bal, 500_000 * ONE);
    assert_eq!(idle(&f), 500_000 * ONE);
    assert_eq!(rwa_held(&f), 0);
    assert_eq!(f.strategy().balance(&vault), 500_000 * ONE);
}

#[test]
fn deposit_rejects_non_positive_non_allowlisted_and_paused() {
    let f = setup(cfg_aqua7());
    let vault = Address::generate(&f.env);
    soroban_sdk::token::StellarAssetClient::new(&f.env, &f.usdc).mint(&vault, &(100 * ONE));
    assert_eq!(f.strategy().try_deposit(&(100 * ONE), &vault), Err(Ok(StrategyError::NotAuthorized)));
    f.strategy().add_depositor(&vault);
    assert_eq!(f.strategy().try_deposit(&0, &vault), Err(Ok(StrategyError::OnlyPositiveAmountAllowed)));
    f.strategy().pause_deposits(&f.keeper, &true);
    assert_eq!(f.strategy().try_deposit(&(100 * ONE), &vault), Err(Ok(StrategyError::NotAuthorized)));
}

#[test]
fn deposit_via_allowlisted_wasm_hash_succeeds_and_unlisted_hash_fails() {
    let f = setup(cfg_aqua7());
    // A vault CONTRACT calls deposit, so the strategy's USDC pull is authorized by a
    // contract inside the call stack, not by the root invoker: plain `mock_all_auths`
    // rejects it (Error(Auth, InvalidAction)).
    f.env.mock_all_auths_allowing_non_root_auth();
    let vault_id = f.env.register(MockVault, ());
    soroban_sdk::token::StellarAssetClient::new(&f.env, &f.usdc).mint(&vault_id, &(100 * ONE));
    let mv = MockVaultClient::new(&f.env, &vault_id);
    assert!(mv.try_go_deposit(&f.strategy_id, &(50 * ONE)).is_err());
    let hash = wasm_hash(&vault_id);
    f.strategy().add_allowed_hash(&hash);
    assert_eq!(mv.go_deposit(&f.strategy_id, &(50 * ONE)), 50 * ONE);
    f.strategy().remove_allowed_hash(&hash);
    assert!(mv.try_go_deposit(&f.strategy_id, &(50 * ONE)).is_err());
}

#[test]
fn deposit_refused_outside_live_while_rwa_is_held() {
    let f = setup(cfg_aqua7());
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 1_000 * ONE);
    mint_rwa_to_strategy(&f, 10 * ONE); // held > 0 so the gate applies
    f.strategy().deposit(&(100 * ONE), &vault); // Live, anchors
    // The pool far under NAV or dead changes nothing: still Live.
    f.aqua().set_price(&6_000_000);
    f.strategy().deposit(&(100 * ONE), &vault);
    f.aqua().set_panic(&true);
    f.strategy().deposit(&(100 * ONE), &vault);
    // OracleDown
    f.sep40().set_panic(&true);
    f.redstone().set_panic(&true);
    assert_eq!(f.strategy().get_pricing_state().mode, PricingMode::OracleDown);
    assert_eq!(f.strategy().try_deposit(&(100 * ONE), &vault), Err(Ok(StrategyError::ExternalError)));
    // Frozen
    f.sep40().set_panic(&false);
    f.redstone().set_panic(&false);
    f.strategy().freeze_price(&f.keeper, &true);
    assert_eq!(f.strategy().get_pricing_state().mode, PricingMode::Frozen);
    assert_eq!(f.strategy().try_deposit(&(100 * ONE), &vault), Err(Ok(StrategyError::ExternalError)));
    f.strategy().freeze_price(&f.admin, &false);
    f.strategy().deposit(&(100 * ONE), &vault);
}

#[test]
fn deposit_with_nothing_held_needs_no_source() {
    let f = setup(cfg_aqua7());
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 100 * ONE);
    f.sep40().set_panic(&true);
    f.redstone().set_panic(&true);
    f.aqua().set_panic(&true);
    assert_eq!(f.strategy().deposit(&(100 * ONE), &vault), 100 * ONE);
}

#[test]
fn balance_returns_in_every_mode_once_shares_exist() {
    let f = setup(cfg_aqua7());
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 100 * ONE);
    f.strategy().deposit(&(100 * ONE), &vault);
    mint_rwa_to_strategy(&f, 100 * ONE); // NAV 200 at 1.00
    assert_near(f.strategy().balance(&vault), 200 * ONE, 10);
    f.aqua().set_price(&6_000_000); // the pool is not an input: still 1.00
    assert_near(f.strategy().balance(&vault), 200 * ONE, 10);
    f.sep40().set_panic(&true);
    f.redstone().set_panic(&true); // OracleDown at anchor 1.00
    assert_near(f.strategy().balance(&vault), 200 * ONE, 10);
    f.sep40().set_panic(&false);
    f.redstone().set_panic(&false);
    f.strategy().freeze_price(&f.keeper, &true); // Frozen at anchor 1.00
    set_all_prices(&f, 9_500_000);
    assert_near(f.strategy().balance(&vault), 200 * ONE, 10);
    f.strategy().freeze_price(&f.admin, &false); // Live: 0.95 floored at L * 0.99
    assert_near(f.strategy().balance(&vault), 199 * ONE, 10);
}

#[test]
fn vault_share_price_ignores_a_pushed_pool() {
    // the pool is not a pricing input, so pool -40% moves NAV by nothing.
    let f = setup(cfg_aqua7());
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 100 * ONE);
    f.strategy().deposit(&(100 * ONE), &vault);
    mint_rwa_to_strategy(&f, 100 * ONE);
    let before = f.strategy().balance(&vault);
    f.aqua().set_price(&6_000_000);
    let after = f.strategy().balance(&vault);
    assert_eq!(after, before);
}

#[test]
fn auto_invest_off_buys_nothing_and_on_buys_exactly_one_tranche() {
    let f = setup(cfg_aqua7());
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 100_000 * ONE);
    f.strategy().deposit(&(30_000 * ONE), &vault);
    assert_eq!(idle(&f), 30_000 * ONE);
    f.strategy().set_auto_invest(&true);
    f.strategy().deposit(&(30_000 * ONE), &vault);
    // one tranche = min(deposit 30_000, max_swap 10_000)
    assert_eq!(idle(&f), 50_000 * ONE);
    assert_eq!(rwa_held(&f), 10_000 * ONE);
    // trailing balance is from the pre-swap NAV
    assert_near(f.strategy().balance(&vault), 60_000 * ONE, 10);
}

#[test]
fn auto_invest_tranche_skips_silently_when_paused_degraded_quote_dead_or_quote_short() {
    let f = setup(cfg_aqua7());
    f.strategy().set_auto_invest(&true);
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 100_000 * ONE);
    // Quote under the floor: with slippage 0 the floor is the oracle-implied quantity, and a
    // pool 0.5% above the oracle quotes 0.5% fewer RWA -> skip.
    let g = setup(Cfg { slippage_bps: 0, ..cfg_aqua7() });
    g.strategy().set_auto_invest(&true);
    let gv = Address::generate(&g.env);
    fund(&g, &gv, 1_000 * ONE);
    g.aqua().set_price(&10_050_000);
    g.strategy().deposit(&(1_000 * ONE), &gv);
    assert_eq!(rwa_held(&g), 0, "quote under the floor must skip, not revert");
    assert_eq!(idle(&g), 1_000 * ONE);

    // Degraded: hold RWA, kill the oracles. (Paused is refused by deposit itself; covered in
    // deposit_rejects_non_positive_non_allowlisted_and_paused.)
    mint_rwa_to_strategy(&f, ONE);
    f.strategy().deposit(&(1_000 * ONE), &vault); // anchors, buys one tranche of 1_000
    let held = rwa_held(&f);
    f.aqua().set_price(&PRICE_ONE);
    f.sep40().set_panic(&true);
    f.redstone().set_panic(&true);
    // OracleDown: deposit refused while held (fail closed), nothing bought.
    assert!(f.strategy().try_deposit(&(1_000 * ONE), &vault).is_err());
    assert_eq!(rwa_held(&f), held);

    // Quote failing with nothing held: the deposit succeeds (par minting needs no source)
    // and the tranche skips because the quote is `None`.
    let h = setup(cfg_aqua7());
    h.strategy().set_auto_invest(&true);
    let hv = Address::generate(&h.env);
    fund(&h, &hv, 1_000 * ONE);
    h.aqua().set_panic(&true);
    assert_eq!(h.strategy().deposit(&(1_000 * ONE), &hv), 1_000 * ONE);
    assert_eq!(rwa_held(&h), 0);
    assert_eq!(idle(&h), 1_000 * ONE);
}

#[test]
fn auto_invest_tranche_is_sized_from_the_deposit_not_the_idle_buffer() {
    // sized from aggregate idle, a 10 USDC deposit bought 1_010 USDC of RWA
    // and spent the 1_000 buffer earlier deposits had left idle for withdrawals.
    for cfg in [cfg_aqua7(), cfg_sushi18()] {
        let f = setup(cfg);
        let vault = Address::generate(&f.env);
        fund(&f, &vault, 1_010 * ONE);
        f.strategy().deposit(&(1_000 * ONE), &vault); // auto_invest off: stays idle as the buffer
        f.strategy().set_auto_invest(&true);
        f.strategy().deposit(&(10 * ONE), &vault);
        assert_eq!(rwa_held(&f), 10 * f.one_rwa(), "only the deposit is invested");
        assert_eq!(idle(&f), 1_000 * ONE, "the existing buffer is untouched");
    }
}

#[test]
fn auto_invest_tranche_respects_max_invested() {
    let f = setup(Cfg { max_invested: 5_000 * ONE, ..cfg_aqua7() });
    f.strategy().set_auto_invest(&true);
    let vault = Address::generate(&f.env);
    fund(&f, &vault, 20_001 * ONE); // the 1 ONE second deposit needs USDC left over
    f.strategy().deposit(&(20_000 * ONE), &vault);
    assert_eq!(rwa_held(&f), 5_000 * ONE);
    f.strategy().deposit(&(1 * ONE), &vault);
    assert_eq!(rwa_held(&f), 5_000 * ONE, "cap reached: no second tranche");
}

#[test]
fn set_auto_invest_is_admin_only_and_emits() {
    let f = setup(cfg_aqua7());
    // mock_all_auths satisfies admin.require_auth(); the role is the signer, so the
    // negative case is proven in the integration suite's real-auth test (Task 18).
    f.strategy().set_auto_invest(&true);
    // Sampled here: `events().all()` reports only the last top-level invocation, and the
    // `get_params` call below is one.
    let emitted = f.env.events().all().events().len();
    assert!(f.strategy().get_params().auto_invest);
    assert_eq!(emitted, 1);
}

#[test]
fn inflation_attack_resisted() {
    // A tiny first deposit plus a donation cannot zero a victim's mint.
    let f = setup(cfg_aqua7());
    let attacker = Address::generate(&f.env);
    let victim = Address::generate(&f.env);
    fund(&f, &attacker, 1_000_000 * ONE);
    fund(&f, &victim, 1_000 * ONE);
    f.strategy().deposit(&1, &attacker);
    // donate RWA outside the accounting
    mint_rwa_to_strategy(&f, 10_000 * ONE);
    let victim_bal = f.strategy().deposit(&(1_000 * ONE), &victim);
    assert!(victim_bal >= 999 * ONE, "victim keeps their value, got {}", victim_bal);
    let _ = TokenClient::new(&f.env, &f.usdc);
}

#[test]
fn max_rwa_in_rounds_up_the_slippage_cap() {
    // The sell-side cap is a quantity the strategy GIVES AWAY, so it must round up: a cap one
    // raw unit short of the implied quantity plus slippage turns an exact fill into an
    // `InternalSwapError`. Arithmetic under `cfg_aqua7()` (7-dp RWA, `slippage_bps` 100):
    //   rate      = 1.00 USDC per whole RWA = PRICE_ONE * SCALAR_12 = 1e7 * 1e12 = 1e19
    //   usdc_out  = 1_000_003 USDC stroops
    //   qty_for   = 1_000_003 * (1e7 * 1e12) / 1e19 = 1_000_003 raw RWA units
    //   * 10_100  = 10_100_030_300, / 10_000 = 1_010_003.03 (not an integer)
    //   floor     = 1_010_003 (what `apply_bps` gives) ... one unit short
    //   ceil      = 1_010_004 (what `apply_bps_ceil` gives) ... the cap we want
    let f = setup(cfg_aqua7());
    let config = f.config();
    let params = f.strategy().get_params();
    assert_eq!(params.slippage_bps, 100, "the fixture's slippage the arithmetic assumes");
    let rate = PRICE_ONE * crate::constants::SCALAR_12;
    let usdc_out = 1_000_003i128;

    let (capped, implied) = f.env.as_contract(&f.strategy_id, || {
        let implied = crate::math::qty_for(&f.env, usdc_out, rate, config.rwa_decimals).unwrap();
        let capped = crate::flows::max_rwa_in(&f.env, &config, &params, rate, usdc_out).unwrap();
        (capped, implied)
    });

    assert_eq!(implied, 1_000_003, "the guarded-rate-implied quantity at a 1.00 rate");
    assert_eq!(capped, 1_010_004, "the cap is the CEILING of implied * 1.01");
    assert_eq!(
        capped,
        crate::math::apply_bps(implied, params.slippage_bps as i128).unwrap() + 1,
        "exactly one raw RWA unit above what flooring would have given"
    );
}
