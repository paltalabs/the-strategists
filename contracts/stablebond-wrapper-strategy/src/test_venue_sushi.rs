#![cfg(test)]
extern crate std;

use soroban_sdk::token::{StellarAssetClient, TokenClient};

use crate::testutils::*;
use crate::venue;
use crate::StrategyError;

fn probe(f: &Fx) -> (soroban_sdk::Address, VenueProbeClient<'_>) {
    let id = f.env.register(VenueProbe, ());
    StellarAssetClient::new(&f.env, &f.usdc).mint(&id, &(1_000_000 * ONE));
    f.rwa_token().mint(&id, &(1_000_000 * f.one_rwa()));
    (id.clone(), VenueProbeClient::new(&f.env, &id))
}

#[test]
fn sushi_quotes_both_directions_with_18dp_rwa() {
    let f = setup(cfg_sushi18());
    f.sushi().set_price(&10_367_000); // 1.0367 USDC per deJTRSY
    assert_eq!(crate::venue_sushi::try_quote(&f.env, &f.sushi_pool, true, 100 * ONE_18), Some(1_036_700_000));
    assert_eq!(venue::try_quote_buy(&f.env, &f.venue, 1_036_700_000), Some(100 * ONE_18));
}

#[test]
fn sushi_quote_is_none_when_paused_or_panicking() {
    let f = setup(cfg_sushi18());
    f.sushi().set_panic(&true);
    assert_eq!(crate::venue_sushi::try_quote(&f.env, &f.sushi_pool, true, ONE_18), None);
    assert_eq!(venue::try_quote_buy(&f.env, &f.venue, ONE), None);
}

#[test]
fn sushi_sell_token0_and_buy_token0_move_tokens() {
    let f = setup(cfg_sushi18());
    let (id, p) = probe(&f);
    let usdc_before = TokenClient::new(&f.env, &f.usdc).balance(&id);
    let out = p.sell(&f.config(), &f.venue, &(100 * ONE_18), &(100 * ONE));
    assert_eq!(out, 100 * ONE);
    assert_eq!(TokenClient::new(&f.env, &f.usdc).balance(&id), usdc_before + 100 * ONE);
    let got = p.buy(&f.config(), &f.venue, &(50 * ONE), &(50 * ONE_18));
    assert_eq!(got, 50 * ONE_18);
}

#[test]
fn sushi_rwa_as_token1_direction_also_works() {
    // Same mock pool, but tell the strategy the RWA is token1: the buy becomes zero_for_one.
    let f = setup(cfg_sushi18());
    let pool = f.env.register(MockSushiPool, (f.usdc.clone(), f.rwa.clone(), f.usdc.clone(), 18u32, PRICE_ONE));
    StellarAssetClient::new(&f.env, &f.usdc).mint(&pool, &(1_000_000 * ONE));
    f.rwa_token().mint(&pool, &(1_000_000 * ONE_18));
    let pool_id = pool.clone();
    let venue = crate::storage::Venue::Sushi(pool, false);
    let (_id, p) = probe(&f);
    assert_eq!(crate::venue_sushi::try_quote(&f.env, &pool_id, false, 10 * ONE_18), Some(10 * ONE));
    assert_eq!(p.sell(&f.config(), &venue, &(10 * ONE_18), &(10 * ONE)), 10 * ONE);
    assert_eq!(p.buy(&f.config(), &venue, &(10 * ONE), &(10 * ONE_18)), 10 * ONE_18);
}

#[test]
fn sushi_short_fill_and_panic_are_typed_swap_errors() {
    let f = setup(cfg_sushi18());
    let (_id, p) = probe(&f);
    f.sushi().set_fill_bps(&9_900);
    // The probe propagates the Result, so the typed error is observable on the wire.
    assert_eq!(
        p.try_sell(&f.config(), &f.venue, &(100 * ONE_18), &(100 * ONE)),
        Err(Ok(StrategyError::InternalSwapError))
    );
    assert_eq!(p.sell(&f.config(), &f.venue, &(100 * ONE_18), &(99 * ONE)), 99 * ONE);
    f.sushi().set_panic(&true);
    assert_eq!(p.try_sell(&f.config(), &f.venue, &ONE_18, &1), Err(Ok(StrategyError::InternalSwapError)));
}

#[test]
fn grant_before_hints_fails_the_swap() {
    // The grant is scoped to the next invocation's subtree; reading hints first consumes it.
    let f = setup(cfg_sushi18());
    let id = f.env.register(SushiMisorderedProbe, ());
    f.rwa_token().mint(&id, &(100 * ONE_18));
    let p = SushiMisorderedProbeClient::new(&f.env, &id);
    assert!(p.try_swap_grant_before_hints(&f.sushi_pool, &f.rwa, &true, &(10 * ONE_18)).is_err());
}
