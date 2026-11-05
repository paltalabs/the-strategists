//! Oracle adapters. `validate_oracles` is shared by the constructor and `propose_oracles`;
//! `read_rate` (Task 8) turns one source into `Option<rate>` with every call a `try_`.

use crate::abi::RedStoneClient;
use crate::constants::{MAX_DECIMALS, MAX_ORACLES, MAX_RATE, MAX_TTL_SECONDS};
use crate::math;
use crate::storage::{Config, OracleKind, OracleSource};
use defindex_strategy_core::StrategyError;
use sep_40_oracle::Asset;
use sep_40_oracle::PriceFeedClient;
use soroban_sdk::{Env, Vec};

/// Live `decimals()` of a source. `Err` when unreachable or malformed. This is the
/// reachability check the constructor and `propose_oracles` rely on.
fn live_decimals(e: &Env, kind: &OracleKind) -> Result<u32, StrategyError> {
    match kind {
        OracleKind::Sep40(feed, _asset) => PriceFeedClient::new(e, feed)
            .try_decimals()
            .map_err(|_| StrategyError::InvalidArgument)?
            .map_err(|_| StrategyError::InvalidArgument),
        OracleKind::RedStone(adapter) => {
            let d = RedStoneClient::new(e, adapter)
                .try_decimals()
                .map_err(|_| StrategyError::InvalidArgument)?
                .map_err(|_| StrategyError::InvalidArgument)?;
            u32::try_from(d).map_err(|_| StrategyError::InvalidArgument)
        }
    }
}

/// Count in `1..=MAX_ORACLES`, every TTL in `(0, MAX_TTL_SECONDS]`, every `decimals` at most
/// `MAX_DECIMALS` and equal to the source's live `decimals()`.
pub fn validate_oracles(e: &Env, oracles: &Vec<OracleSource>) -> Result<(), StrategyError> {
    let n = oracles.len();
    if n == 0 || n > MAX_ORACLES {
        return Err(StrategyError::InvalidArgument);
    }
    for source in oracles.iter() {
        if source.ttl_seconds == 0 || source.ttl_seconds > MAX_TTL_SECONDS {
            return Err(StrategyError::InvalidArgument);
        }
        if source.decimals > MAX_DECIMALS {
            return Err(StrategyError::InvalidArgument);
        }
        if live_decimals(e, &source.kind)? != source.decimals {
            return Err(StrategyError::InvalidArgument);
        }
    }
    Ok(())
}

/// `Some(rate)` iff the source answered, its price is positive, its timestamp is within
/// `ttl_seconds` of the ledger clock and the normalized rate is **strictly positive** and at
/// most `MAX_RATE`. A positive price is not automatically a positive rate: `rate_from_price`
/// floors, so a source whose `decimals` would exceed `asset_decimals + 12` would floor to 0 —
/// with `MAX_DECIMALS = 18` enforced at construction (`validate_oracles`, above) and a 7-decimal
/// asset, no accepted source can reach that gap, so the `> 0` filter here is defense in depth
/// against a floored conversion, not a reachable case. It still guards `invest_otc`'s
/// `Live` gate (`pricing::can_buy`) against a 0 either way. RedStone timestamps are
/// milliseconds.
pub fn read_rate(e: &Env, config: &Config, source: &OracleSource) -> Option<i128> {
    let now = e.ledger().timestamp();
    let (price, ts_seconds): (i128, u64) = match &source.kind {
        OracleKind::Sep40(feed, asset) => {
            let pd = PriceFeedClient::new(e, feed)
                .try_lastprice(&Asset::Stellar(asset.clone()))
                .ok()?
                .ok()??;
            (pd.price, pd.timestamp)
        }
        OracleKind::RedStone(adapter) => {
            let pd = RedStoneClient::new(e, adapter).try_read_price_data().ok()?.ok()?;
            let price = i128::try_from(pd.price.to_u128()?).ok()?;
            (price, pd.package_timestamp / 1_000)
        }
    };
    if price <= 0 || now.saturating_sub(ts_seconds) > source.ttl_seconds {
        return None;
    }
    math::rate_from_price(e, price, source.decimals, config.asset_decimals)
        .ok()
        .filter(|r| *r > 0 && *r <= MAX_RATE)
}

/// Execution-time check for a proposed set (`execute_oracles`): at least one proposed
/// source must read live (`read_rate`). Nothing more: the set is not compared with the pool,
/// which sits 100+ bps under NAV for weeks, nor with the anchor, which a wrong oracle may
/// already have walked further than any fixed tolerance before it was frozen, so either check
/// could refuse the correct replacement forever on a contract that cannot be upgraded. A
/// wrong feed swapped in is bounded exactly like any wrong oracle: the anchor clamp
/// and `freeze_price`; the 72 h public wait bounds a compromised admin.
pub fn market_check(e: &Env, config: &Config, proposed: &Vec<OracleSource>) -> Result<(), StrategyError> {
    for source in proposed.iter() {
        if read_rate(e, config, &source).is_some() {
            return Ok(());
        }
    }
    Err(StrategyError::ExternalError)
}
