//! Testnet faucet: anyone can claim a fixed amount of each test token once per cooldown.
//! The test tokens' admin is fixed at deploy, so the faucet does not mint: it hands out a
//! balance the admin minted to it, and is refilled the same way.
#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, token::TokenClient, Address, Env, Vec};

#[contracttype]
enum Key {
    Tokens,
    Amounts,
    Cooldown,
    LastClaim(Address),
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum FaucetError {
    /// The address claimed less than `cooldown` seconds ago.
    TooSoon = 1,
    /// `tokens` and `amounts` differ in length, or an amount is not positive.
    BadConfig = 2,
}

/// Ledgers close about every 5 s; keep a claim record a little longer than the cooldown.
const SECONDS_PER_LEDGER: u64 = 5;

#[contract]
pub struct Faucet;

#[contractimpl]
impl Faucet {
    /// Hands out `amounts[i]` of `tokens[i]` per claim, at most once per `cooldown` seconds.
    pub fn __constructor(e: Env, tokens: Vec<Address>, amounts: Vec<i128>, cooldown: u64) -> Result<(), FaucetError> {
        if tokens.len() != amounts.len() || amounts.iter().any(|a| a <= 0) {
            return Err(FaucetError::BadConfig);
        }
        e.storage().instance().set(&Key::Tokens, &tokens);
        e.storage().instance().set(&Key::Amounts, &amounts);
        e.storage().instance().set(&Key::Cooldown, &cooldown);
        Ok(())
    }

    /// Sends the configured amounts to `to`. `to` signs, so nobody drains it in someone else's name.
    pub fn claim(e: Env, to: Address) -> Result<(), FaucetError> {
        to.require_auth();
        let next = Self::next_claim(e.clone(), to.clone());
        let now = e.ledger().timestamp();
        if now < next {
            return Err(FaucetError::TooSoon);
        }
        let cooldown: u64 = e.storage().instance().get(&Key::Cooldown).unwrap_or(0);
        let key = Key::LastClaim(to.clone());
        e.storage().temporary().set(&key, &now);
        let ledgers = (cooldown / SECONDS_PER_LEDGER + 100) as u32;
        e.storage().temporary().extend_ttl(&key, ledgers, ledgers);
        e.storage().instance().extend_ttl(100_000, 500_000);

        let tokens: Vec<Address> = e.storage().instance().get(&Key::Tokens).unwrap();
        let amounts: Vec<i128> = e.storage().instance().get(&Key::Amounts).unwrap();
        let this = e.current_contract_address();
        for (token, amount) in tokens.iter().zip(amounts.iter()) {
            TokenClient::new(&e, &token).transfer(&this, &to, &amount);
        }
        Ok(())
    }

    /// Unix time from which `to` can claim again (0 if it never claimed).
    pub fn next_claim(e: Env, to: Address) -> u64 {
        let cooldown: u64 = e.storage().instance().get(&Key::Cooldown).unwrap_or(0);
        e.storage()
            .temporary()
            .get::<_, u64>(&Key::LastClaim(to))
            .map_or(0, |last| last + cooldown)
    }

    /// The tokens and the amount of each handed out per claim.
    pub fn config(e: Env) -> (Vec<Address>, Vec<i128>, u64) {
        let s = e.storage().instance();
        (s.get(&Key::Tokens).unwrap(), s.get(&Key::Amounts).unwrap(), s.get(&Key::Cooldown).unwrap_or(0))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::{Address as _, Ledger}, token::StellarAssetClient, vec};

    fn setup() -> (Env, FaucetClient<'static>, Address, Address) {
        let e = Env::default();
        e.mock_all_auths();
        let admin = Address::generate(&e);
        let usdc = e.register_stellar_asset_contract_v2(admin.clone()).address();
        let rwa = e.register_stellar_asset_contract_v2(admin.clone()).address();
        let id = e.register(Faucet, (vec![&e, usdc.clone(), rwa.clone()], vec![&e, 100i128, 10i128], 3600u64));
        StellarAssetClient::new(&e, &usdc).mint(&id, &1_000);
        StellarAssetClient::new(&e, &rwa).mint(&id, &1_000);
        (e.clone(), FaucetClient::new(&e, &id), usdc, rwa)
    }

    #[test]
    fn claim_pays_each_token_then_waits_the_cooldown() {
        let (e, f, usdc, rwa) = setup();
        let user = Address::generate(&e);
        f.claim(&user);
        assert_eq!(TokenClient::new(&e, &usdc).balance(&user), 100);
        assert_eq!(TokenClient::new(&e, &rwa).balance(&user), 10);
        assert_eq!(f.try_claim(&user), Err(Ok(FaucetError::TooSoon)));
        e.ledger().with_mut(|l| l.timestamp += 3600);
        f.claim(&user);
        assert_eq!(TokenClient::new(&e, &usdc).balance(&user), 200);
    }

    #[test]
    fn cooldown_is_per_address() {
        let (e, f, usdc, _) = setup();
        let (a, b) = (Address::generate(&e), Address::generate(&e));
        f.claim(&a);
        f.claim(&b);
        assert_eq!(TokenClient::new(&e, &usdc).balance(&b), 100);
        assert_eq!(f.next_claim(&a), e.ledger().timestamp() + 3600);
    }

    #[test]
    #[should_panic]
    fn claim_needs_the_receivers_signature() {
        let e = Env::default();
        let admin = Address::generate(&e);
        let usdc = e.register_stellar_asset_contract_v2(admin).address();
        let id = e.register(Faucet, (vec![&e, usdc], vec![&e, 100i128], 0u64));
        FaucetClient::new(&e, &id).claim(&Address::generate(&e));
    }

    #[test]
    #[should_panic]
    fn mismatched_config_is_refused() {
        let e = Env::default();
        let t = Address::generate(&e);
        e.register(Faucet, (vec![&e, t], vec![&e, 1i128, 2i128], 0u64));
    }
}
