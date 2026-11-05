#![no_std]
//! Testnet-only SEP-41 token with an admin mint. Contract tokens need no trustlines, so any
//! account (including fee receivers we do not control) can receive them.
use soroban_sdk::{contract, contractimpl, contracttype, token::TokenInterface, Address, Env, MuxedAddress, String};

const BUMP: u32 = 518_400; // ~30 days
const THRESHOLD: u32 = 432_000;

#[contracttype]
enum Key {
    Admin,
    Decimals,
    Name,
    Symbol,
    Balance(Address),
    Allowance(Address, Address),
}

#[contracttype]
#[derive(Clone)]
struct AllowanceValue {
    amount: i128,
    expiration_ledger: u32,
}

fn bump_instance(e: &Env) {
    e.storage().instance().extend_ttl(THRESHOLD, BUMP);
}

fn balance_of(e: &Env, id: &Address) -> i128 {
    let k = Key::Balance(id.clone());
    let v: i128 = e.storage().persistent().get(&k).unwrap_or(0);
    if v != 0 {
        e.storage().persistent().extend_ttl(&k, THRESHOLD, BUMP);
    }
    v
}

fn set_balance(e: &Env, id: &Address, v: i128) {
    let k = Key::Balance(id.clone());
    e.storage().persistent().set(&k, &v);
    e.storage().persistent().extend_ttl(&k, THRESHOLD, BUMP);
}

fn move_funds(e: &Env, from: &Address, to: &Address, amount: i128) {
    if amount < 0 {
        panic!("negative amount");
    }
    let fb = balance_of(e, from);
    if fb < amount {
        panic!("insufficient balance");
    }
    set_balance(e, from, fb - amount);
    set_balance(e, to, balance_of(e, to) + amount);
}

fn allowance_of(e: &Env, from: &Address, spender: &Address) -> AllowanceValue {
    let v: Option<AllowanceValue> = e.storage().temporary().get(&Key::Allowance(from.clone(), spender.clone()));
    match v {
        Some(a) if a.expiration_ledger >= e.ledger().sequence() => a,
        _ => AllowanceValue { amount: 0, expiration_ledger: 0 },
    }
}

fn spend_allowance(e: &Env, from: &Address, spender: &Address, amount: i128) {
    let a = allowance_of(e, from, spender);
    if a.amount < amount {
        panic!("insufficient allowance");
    }
    let k = Key::Allowance(from.clone(), spender.clone());
    e.storage().temporary().set(&k, &AllowanceValue { amount: a.amount - amount, expiration_ledger: a.expiration_ledger });
}

#[contract]
pub struct TestToken;

#[contractimpl]
impl TestToken {
    pub fn __constructor(e: Env, admin: Address, decimals: u32, name: String, symbol: String) {
        e.storage().instance().set(&Key::Admin, &admin);
        e.storage().instance().set(&Key::Decimals, &decimals);
        e.storage().instance().set(&Key::Name, &name);
        e.storage().instance().set(&Key::Symbol, &symbol);
    }

    pub fn mint(e: Env, to: Address, amount: i128) {
        let admin: Address = e.storage().instance().get(&Key::Admin).unwrap();
        admin.require_auth();
        bump_instance(&e);
        if amount < 0 {
            panic!("negative amount");
        }
        set_balance(&e, &to, balance_of(&e, &to) + amount);
    }
}

#[contractimpl]
impl TokenInterface for TestToken {
    fn allowance(e: Env, from: Address, spender: Address) -> i128 {
        allowance_of(&e, &from, &spender).amount
    }

    fn approve(e: Env, from: Address, spender: Address, amount: i128, expiration_ledger: u32) {
        from.require_auth();
        bump_instance(&e);
        let k = Key::Allowance(from, spender);
        e.storage().temporary().set(&k, &AllowanceValue { amount, expiration_ledger });
        if amount > 0 && expiration_ledger >= e.ledger().sequence() {
            let live = expiration_ledger - e.ledger().sequence();
            e.storage().temporary().extend_ttl(&k, live, live);
        }
    }

    fn balance(e: Env, id: Address) -> i128 {
        bump_instance(&e);
        balance_of(&e, &id)
    }

    fn transfer(e: Env, from: Address, to: MuxedAddress, amount: i128) {
        from.require_auth();
        bump_instance(&e);
        move_funds(&e, &from, &to.address(), amount);
    }

    fn transfer_from(e: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();
        bump_instance(&e);
        spend_allowance(&e, &from, &spender, amount);
        move_funds(&e, &from, &to, amount);
    }

    fn burn(e: Env, from: Address, amount: i128) {
        from.require_auth();
        bump_instance(&e);
        let b = balance_of(&e, &from);
        if amount < 0 || b < amount {
            panic!("insufficient balance");
        }
        set_balance(&e, &from, b - amount);
    }

    fn burn_from(e: Env, spender: Address, from: Address, amount: i128) {
        spender.require_auth();
        bump_instance(&e);
        spend_allowance(&e, &from, &spender, amount);
        let b = balance_of(&e, &from);
        if amount < 0 || b < amount {
            panic!("insufficient balance");
        }
        set_balance(&e, &from, b - amount);
    }

    fn decimals(e: Env) -> u32 {
        e.storage().instance().get(&Key::Decimals).unwrap()
    }

    fn name(e: Env) -> String {
        e.storage().instance().get(&Key::Name).unwrap()
    }

    fn symbol(e: Env) -> String {
        e.storage().instance().get(&Key::Symbol).unwrap()
    }
}
