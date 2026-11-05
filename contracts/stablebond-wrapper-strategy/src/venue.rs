//! Venue boundary. The flows call only the functions here; the pool-specific wiring lives
//! in `venue_aqua.rs` and `venue_sushi.rs`.

use crate::abi::{AquaPoolClient, SushiPoolClient};
use crate::storage::{Config, Venue};
use crate::venue_aqua;
use crate::venue_sushi;
use defindex_strategy_core::StrategyError;
use soroban_sdk::Env;
use soroban_sdk::{
    auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation},
    vec, Address, IntoVal, Symbol,
};

/// The pool must hold `rwa_asset` and `asset` where the venue says they are.
pub fn validate_venue(e: &Env, config: &Config, venue: &Venue) -> Result<(), StrategyError> {
    match venue {
        Venue::Aqua(pool, rwa_idx, usdc_idx) => {
            if rwa_idx == usdc_idx {
                return Err(StrategyError::InvalidArgument);
            }
            let tokens = AquaPoolClient::new(e, pool)
                .try_get_tokens()
                .map_err(|_| StrategyError::InvalidArgument)?
                .map_err(|_| StrategyError::InvalidArgument)?;
            if tokens.get(*rwa_idx) != Some(config.rwa_asset.clone())
                || tokens.get(*usdc_idx) != Some(config.asset.clone())
            {
                return Err(StrategyError::InvalidArgument);
            }
            Ok(())
        }
        Venue::Sushi(pool, rwa_is_token0) => {
            let client = SushiPoolClient::new(e, pool);
            let t0 = client
                .try_token0()
                .map_err(|_| StrategyError::InvalidArgument)?
                .map_err(|_| StrategyError::InvalidArgument)?;
            let t1 = client
                .try_token1()
                .map_err(|_| StrategyError::InvalidArgument)?
                .map_err(|_| StrategyError::InvalidArgument)?;
            let (rwa, usdc) = if *rwa_is_token0 { (t0, t1) } else { (t1, t0) };
            if rwa != config.rwa_asset || usdc != config.asset {
                return Err(StrategyError::InvalidArgument);
            }
            Ok(())
        }
    }
}

/// Grants exactly one `token.transfer(me, spender, amount)` anywhere in the subtree of the
/// NEXT invocation. Always the statement right before the venue call.
pub fn grant_transfer(e: &Env, me: &Address, token: &Address, spender: &Address, amount: i128) {
    e.authorize_as_current_contract(vec![
        e,
        InvokerContractAuthEntry::Contract(SubContractInvocation {
            context: ContractContext {
                contract: token.clone(),
                fn_name: Symbol::new(e, "transfer"),
                args: (me.clone(), spender.clone(), amount).into_val(e),
            },
            sub_invocations: vec![e],
        }),
    ]);
}

/// RWA out for buying with `usdc_in` USDC. `None` when the venue is not live.
pub fn try_quote_buy(e: &Env, venue: &Venue, usdc_in: i128) -> Option<i128> {
    match venue {
        Venue::Aqua(pool, rwa_idx, usdc_idx) => venue_aqua::try_quote(e, pool, *usdc_idx, *rwa_idx, usdc_in),
        Venue::Sushi(pool, rwa_is_token0) => venue_sushi::try_quote(e, pool, !*rwa_is_token0, usdc_in),
    }
}

/// Sells `rwa_in` RWA for at least `min_out` USDC.
pub fn sell(e: &Env, config: &Config, venue: &Venue, rwa_in: i128, min_out: i128) -> Result<i128, StrategyError> {
    match venue {
        Venue::Aqua(pool, rwa_idx, usdc_idx) => {
            venue_aqua::swap(e, pool, &config.rwa_asset, *rwa_idx, *usdc_idx, rwa_in, min_out)
        }
        Venue::Sushi(pool, rwa_is_token0) => {
            venue_sushi::swap(e, pool, &config.rwa_asset, *rwa_is_token0, rwa_in, min_out)
        }
    }
}

/// Buys RWA with `usdc_in` USDC, requiring at least `min_out` RWA.
pub fn buy(e: &Env, config: &Config, venue: &Venue, usdc_in: i128, min_out: i128) -> Result<i128, StrategyError> {
    match venue {
        Venue::Aqua(pool, rwa_idx, usdc_idx) => {
            venue_aqua::swap(e, pool, &config.asset, *usdc_idx, *rwa_idx, usdc_in, min_out)
        }
        Venue::Sushi(pool, rwa_is_token0) => {
            venue_sushi::swap(e, pool, &config.asset, !*rwa_is_token0, usdc_in, min_out)
        }
    }
}
