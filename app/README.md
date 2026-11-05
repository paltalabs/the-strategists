# The Strategists · RWA Yield-Sharing (Testnet)

A small dApp for Stellar testnet. It shows a DeFindex vault that earns from one RWA yield
strategy, with the vault state, the strategy's pricing and holdings, and how gains are split.
A connected wallet can deposit and withdraw.

## Run

```bash
npm i
npm run dev      # http://localhost:5173
npm run build    # type-check + production build into dist/
```

You need a Stellar wallet on **Testnet** (Freighter or any wallet the Stellar Wallets Kit
lists). Fund the account with testnet XLM from Friendbot. Test USDC is not available from a
faucet here: ask the team for some. A classic-asset token needs a trustline first; when the
USDC token is a classic asset (its `name()` reads `CODE:ISSUER`), the app shows an
"Add trustline" button.

## Contract addresses

The app reads `../testnet.contracts.json` (the repo root, next to `app/`) at build time:

```json
{
  "usdc": "C...", "rwa": "C...", "oracle": "C...", "pool": "C...",
  "strategy": "C...", "vault": "C...",
  "partner": "G...", "defindex_receiver": "G..."
}
```

`src/config.ts` loads it with `import.meta.glob`, so a missing file is not a build error.
If the file is missing or lacks a key, the page shows "Not deployed yet" with the
missing keys. Extra keys are ignored. After the file changes, restart `npm run dev` or
rebuild.

## What it reads and calls

Contract specs are fetched from chain with `contract.Client.from` (no generated bindings).
RPC: `https://soroban-testnet.stellar.org`.

| Contract | Method | Use |
|---|---|---|
| Vault | `fetch_total_managed_funds()` | TVL, idle and invested amounts (read-only version, does not lock fees) |
| Vault | `get_fees()` | `(vault_fee, protocol_rate)` in bps, shown as the revenue split |
| Vault | `get_fee_receiver()` | fee receiver address |
| Vault | `total_supply()`, `balance(id)` | share supply and the user's shares |
| Vault | `report()` | **simulated only**, on the "Preview fees" button: gains and locked fees |
| Vault | `deposit(amounts_desired, amounts_min, from, invest=false)` | deposit (signed) |
| Vault | `withdraw(withdraw_shares, min_amounts_out, from)` | withdraw (signed), min out = current value − 1% |
| Strategy | `get_pricing_state()`, `get_config()`, `get_params()`, `balance(from=vault)` | mode, rate, decimals, venue, caps |
| Strategy | instance storage `LastValidPrice`, `AnchorTs` | the anchor (no getter exists) |
| USDC / RWA tokens | `balance(id)`, `symbol()`, `decimals()`, `name()` | holdings and labels |

The revenue split: depositors keep `1 − vault_fee` of gains; the partner gets
`vault_fee × (1 − protocol_rate)` and infrastructure `vault_fee × protocol_rate`.

The rate is `price × 10^usdc_decimals × 10^12` (USD per whole RWA), so the UI divides by
`10^(usdc_decimals + 12)`. The page refreshes every 15 seconds.

## Files

- `src/config.ts`: network constants and the contract addresses
- `src/chain.ts`: RPC, contract clients, simulation, signed submission, trustline
- `src/wallet.ts`: Stellar Wallets Kit setup (testnet) and signing
- `src/data.ts`: loaders for the vault, strategy and position
- `src/App.tsx`, `src/*Card.tsx`, `src/ui.tsx`: the page
