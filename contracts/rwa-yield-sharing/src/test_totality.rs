#![cfg(test)]
//! Spike: a `try_` call to an address with no contract behind it must surface
//! as `Err` inside the calling contract, or `balance()` is not total. Kept as a regression
//! test because the whole design rests on the answer.

extern crate std;

use sep_40_oracle::PriceFeedClient;
use soroban_sdk::{contract, contractimpl, testutils::Address as _, Address, Env};

#[contract]
pub struct TotalityProbe;

#[contractimpl]
impl TotalityProbe {
    /// True when the `try_` call returned `Err` instead of trapping the invocation.
    pub fn probe(e: Env, target: Address) -> bool {
        PriceFeedClient::new(&e, &target).try_decimals().is_err()
    }
}

#[test]
fn try_call_to_absent_contract_returns_err_inside_the_caller() {
    let env = Env::default();
    let probe = env.register(TotalityProbe, ());
    let absent = Address::generate(&env);
    let client = TotalityProbeClient::new(&env, &probe);
    // The outer try_ separates "the probe returned" from "the probe itself trapped".
    // `Ok(Ok(true))` = degraded inside the caller. Anything else = the trap escaped the
    // inner try_ and would take a vault's plain `balance()` call down with it.
    assert_eq!(client.try_probe(&absent), Ok(Ok(true)));
}
