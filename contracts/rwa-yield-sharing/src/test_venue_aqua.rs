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
fn aqua_quotes_both_directions_at_the_pool_price() {
    let f = setup(cfg_aqua7());
    f.aqua().set_price(&10_500_000); // 1.05
    assert_eq!(crate::venue_aqua::try_quote(&f.env, &f.aqua_pool, 1, 0, 100 * ONE), Some(105 * ONE));
    assert_eq!(venue::try_quote_buy(&f.env, &f.venue, 105 * ONE), Some(100 * ONE));
}

#[test]
fn aqua_quote_is_none_when_the_pool_panics_or_the_amount_is_bad() {
    let f = setup(cfg_aqua7());
    assert_eq!(crate::venue_aqua::try_quote(&f.env, &f.aqua_pool, 1, 0, 0), None);
    assert_eq!(crate::venue_aqua::try_quote(&f.env, &f.aqua_pool, 1, 0, -1), None);
    f.aqua().set_panic(&true);
    assert_eq!(crate::venue_aqua::try_quote(&f.env, &f.aqua_pool, 1, 0, ONE), None);
    assert_eq!(venue::try_quote_buy(&f.env, &f.venue, ONE), None);
}

#[test]
fn aqua_sell_and_buy_move_tokens_and_honor_min_out() {
    let f = setup(cfg_aqua7());
    let (id, p) = probe(&f);
    let usdc_before = TokenClient::new(&f.env, &f.usdc).balance(&id);
    let out = p.sell(&f.config(), &f.venue, &(100 * ONE), &(100 * ONE));
    assert_eq!(out, 100 * ONE);
    assert_eq!(TokenClient::new(&f.env, &f.usdc).balance(&id), usdc_before + 100 * ONE);
    let got = p.buy(&f.config(), &f.venue, &(50 * ONE), &(50 * ONE));
    assert_eq!(got, 50 * ONE);
}

#[test]
fn aqua_short_fill_below_min_out_is_a_typed_swap_error() {
    let f = setup(cfg_aqua7());
    let (_id, p) = probe(&f);
    f.aqua().set_fill_bps(&9_800); // pays 2% under the quote
    // The probe propagates the Result, so the typed error is observable on the wire.
    assert_eq!(
        p.try_sell(&f.config(), &f.venue, &(100 * ONE), &(100 * ONE)),
        Err(Ok(StrategyError::InternalSwapError))
    );
    // A tolerant min_out goes through.
    assert_eq!(p.sell(&f.config(), &f.venue, &(100 * ONE), &(98 * ONE)), 98 * ONE);
}

#[test]
fn aqua_swap_against_a_panicking_pool_is_a_typed_swap_error() {
    let f = setup(cfg_aqua7());
    let (_id, p) = probe(&f);
    f.aqua().set_panic(&true);
    assert_eq!(p.try_sell(&f.config(), &f.venue, &ONE, &1), Err(Ok(StrategyError::InternalSwapError)));
}
