#![no_std]
//! Testnet-only stand-in for the Sushi factory. The pool reads its protocol fee from its
//! factory on every quote and swap, so one has to exist; zero fee, as on the mainnet pool.
use soroban_sdk::{contract, contractimpl, Env};

#[contract]
pub struct SushiFactoryStub;

#[contractimpl]
impl SushiFactoryStub {
    pub fn get_protocol_fee_0(_e: Env) -> u32 {
        0
    }
    pub fn get_protocol_fee_1(_e: Env) -> u32 {
        0
    }
}
