//! External contract ABIs the strategy calls. Each trait carries only the functions used.
//! Wire structs are `#[contracttype(export = false)]`: an ScMap is keyed by field name, so
//! the names below are the on-chain format and must not change. Interfaces fetched with
//! `stellar contract info interface` from mainnet:
//!   Aqua pool CCX2TYR4AQTPPMTZOIMP3YIBHAYLLSTXVP47PAGTL753PCQFSKV32MIA (USTRY/USDC),
//!   Sushi pool CDMJRRH5MAJLB7T5SQAWQOOL7UJ3BABGTURSVKIE6AEDSXESDDJIBVCT (deJTRSY/USDC),
//!   RedStone adapter CAEGYZFAJ3HLSDBUIVJ77SK5HLLS4MPE4LO5OB2W3L2TEDMAS6RCTMZ4 (USTRY).
//! On-chain `Result<T, E>` returns are declared as `T`: the host returns the Ok value and
//! fails the invocation on Err, which the `try_` client surfaces as `Err`.

use soroban_sdk::{contractclient, contracttype, Address, Env, Vec, U256};

/// Aqua liquidity pool, called directly (no router).
#[allow(dead_code)]
#[contractclient(name = "AquaPoolClient")]
pub trait AquaPool {
    fn get_tokens(env: Env) -> Vec<Address>;
    fn estimate_swap(env: Env, in_idx: u32, out_idx: u32, in_amount: u128) -> u128;
    fn swap(env: Env, user: Address, in_idx: u32, out_idx: u32, in_amount: u128, out_min: u128) -> u128;
}

/// Sushi `get_oracle_hints()` return and `swap` argument.
#[contracttype(export = false)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleHints {
    pub checkpoint: u32,
    pub checkpoint_min: u32,
    pub slot: u128,
}

/// Sushi `swap` / `quote_exact_input` return: positive = paid into the pool, negative =
/// received from it.
#[contracttype(export = false)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwapResult {
    pub amount0: i128,
    pub amount1: i128,
    pub liquidity: u128,
    pub sqrt_price_x96: U256,
    pub tick: i32,
}

/// Sushi (Uniswap v3 style) concentrated pool.
#[allow(dead_code)]
#[contractclient(name = "SushiPoolClient")]
pub trait SushiPool {
    fn token0(env: Env) -> Address;
    fn token1(env: Env) -> Address;
    fn get_oracle_hints(env: Env) -> OracleHints;
    fn quote_exact_input(env: Env, zero_for_one: bool, amount_in: i128, sqrt_price_limit_x96: U256) -> SwapResult;
    fn swap(
        env: Env,
        sender: Address,
        recipient: Address,
        zero_for_one: bool,
        amount_specified: i128,
        sqrt_price_limit_x96: U256,
        hints: OracleHints,
    ) -> SwapResult;
}

/// RedStone `read_price_data()` return. Timestamps are milliseconds.
#[contracttype(export = false)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedStonePriceData {
    pub package_timestamp: u64,
    pub price: U256,
    pub write_timestamp: u64,
}

/// RedStone per-asset price adapter (not SEP-40).
#[allow(dead_code)]
#[contractclient(name = "RedStoneClient")]
pub trait RedStoneAdapter {
    fn decimals(env: Env) -> u64;
    fn read_price_data(env: Env) -> RedStonePriceData;
}
