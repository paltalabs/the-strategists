#![cfg(test)]
extern crate std;

use soroban_sdk::testutils::Ledger as _;

use crate::constants::{MAX_RATE, SCALAR_12};
use crate::oracles::read_rate;
use crate::storage::Config;
use crate::testutils::*;

#[test]
fn sep40_fresh_price_normalizes_from_14dp() {
    let f = setup(cfg_aqua7());
    set_all_prices(&f, 10_745_000); // 1.0745
    let src = f.oracles.get(0).unwrap();
    assert_eq!(read_rate(&f.env, &f.config(), &src), Some(10_745_000 * SCALAR_12));
}

#[test]
fn redstone_fresh_price_normalizes_from_8dp_and_milliseconds() {
    let f = setup(cfg_sushi18());
    set_all_prices(&f, 10_367_000);
    let src = f.oracles.get(0).unwrap();
    assert_eq!(read_rate(&f.env, &f.config(), &src), Some(10_367_000 * SCALAR_12));
}

#[test]
fn stale_sources_are_not_live() {
    let f = setup(cfg_aqua7());
    set_all_prices(&f, PRICE_ONE);
    let sep = f.oracles.get(0).unwrap();
    let red = f.oracles.get(1).unwrap();
    // 901 s later: Sep40 (ttl 900) dead, RedStone (ttl 86_400) alive.
    f.env.ledger().with_mut(|l| l.timestamp = START_TS + 901);
    assert_eq!(read_rate(&f.env, &f.config(), &sep), None);
    assert!(read_rate(&f.env, &f.config(), &red).is_some());
    // 24 h + 1 s later: both dead.
    f.env.ledger().with_mut(|l| l.timestamp = START_TS + 86_401);
    assert_eq!(read_rate(&f.env, &f.config(), &red), None);
}

#[test]
fn zero_missing_and_panicking_sources_are_not_live() {
    let f = setup(cfg_aqua7());
    let sep = f.oracles.get(0).unwrap();
    let red = f.oracles.get(1).unwrap();
    f.sep40().set_price(&0, &now(&f));
    assert_eq!(read_rate(&f.env, &f.config(), &sep), None);
    f.sep40().set_price(&(PRICE_ONE * 10_000_000), &now(&f));
    f.sep40().set_missing(&true);
    assert_eq!(read_rate(&f.env, &f.config(), &sep), None);
    f.sep40().set_missing(&false);
    f.sep40().set_panic(&true);
    assert_eq!(read_rate(&f.env, &f.config(), &sep), None);
    f.redstone().set_price(&0u128, &(now(&f) * 1_000));
    assert_eq!(read_rate(&f.env, &f.config(), &red), None);
    f.redstone().set_panic(&true);
    assert_eq!(read_rate(&f.env, &f.config(), &red), None);
}

#[test]
fn a_price_that_normalizes_to_zero_is_not_live() {
    // `rate_from_price` FLOORS `price * 10^asset_decimals * SCALAR_12 / 10^source_decimals`,
    // so a positive price is not the same thing as a positive rate, and a rate of 0 must not
    // count as live: it would become `min(O)`, put pricing in `Live`, and satisfy
    // `invest_otc`'s `Live` gate (`src/otc.rs`).
    //
    // The fixture's own scale cannot produce it: a 14-dp Sep40 against 7-dp USDC is
    // `rate = price * 10^5`, and `validate_oracles` caps `decimals` at `MAX_DECIMALS = 18`,
    // which against 7-dp USDC still floors no lower than `price * 10`. What is reachable is a
    // low-decimal *underlying*: `asset_decimals` is only bounded ABOVE (`src/lib.rs:128-129`),
    // so the same 14-dp feed read for a 0-decimal asset is `price / 100`.
    let f = setup(cfg_aqua7());
    let sep = f.oracles.get(0).unwrap();
    let zero_dec_asset = Config { asset_decimals: 0, ..f.config() };
    f.sep40().set_price(&99, &now(&f)); // 99 / 100 floors to 0
    assert_eq!(read_rate(&f.env, &zero_dec_asset, &sep), None);
    // One unit higher the rate is 1 and the very same source is live: only the zero is
    // rejected, and the fixture's real 7-dp config never sees the floor at all.
    f.sep40().set_price(&100, &now(&f));
    assert_eq!(read_rate(&f.env, &zero_dec_asset, &sep), Some(1));
    assert_eq!(read_rate(&f.env, &f.config(), &sep), Some(100 * 100_000));
}

#[test]
fn redstone_price_above_i128_is_not_live() {
    let f = setup(cfg_sushi18());
    let red = f.oracles.get(0).unwrap();
    f.redstone().set_price(&u128::MAX, &(now(&f) * 1_000));
    assert_eq!(read_rate(&f.env, &f.config(), &red), None);
}

#[test]
fn a_rate_above_the_ceiling_is_not_live() {
    let f = setup(cfg_aqua7());
    let sep = f.oracles.get(0).unwrap();
    let red = f.oracles.get(1).unwrap();
    // Sep40 reports 14 dp against 7-dp USDC, so `rate = price * 10^(7+12) / 10^14 =
    // price * 10^5`. `4 * MAX_RATE / 10^5` therefore normalizes to ~`4 * MAX_RATE`
    // (~3.40e34): a number that still fits i128 and that `rate_from_price` returns happily,
    // but that `spread_bps` would then multiply by 10_000 into ~3.40e38, past
    // i128::MAX (~1.70e38). The ceiling is what keeps it out of pricing.
    f.sep40().set_price(&(4 * MAX_RATE / 100_000), &now(&f));
    assert_eq!(read_rate(&f.env, &f.config(), &sep), None);
    // Only the ceiling rejects it: a price landing exactly at `MAX_RATE` is still live.
    let at_ceiling = MAX_RATE / 100_000;
    f.sep40().set_price(&at_ceiling, &now(&f));
    assert_eq!(read_rate(&f.env, &f.config(), &sep), Some(at_ceiling * 100_000));
    // The other source, at a normal price, is untouched.
    assert_eq!(read_rate(&f.env, &f.config(), &red), Some(PRICE_ONE * SCALAR_12));
}
