# RWA Yield-Sharing Strategy (USDC-in / USDC-out)

A DeFindex vault **strategy** contract that wraps a value-accruing RWA stablebond
(USTRY / USDY / deJTRSY / CETES) on Stellar/Soroban.

The vault deposits **USDC**; the strategy keeps it idle and buys the RWA with an allowlisted OTC
counterparty or in capped tranches on one fixed Aqua or Sushi pool. Yield accrues as RWA **price
appreciation** and is realized back into USDC on withdraw. There is no reward token and nothing
to claim — `harvest` is a telemetry no-op.

This USDC round-trip is deliberate. A passive RWA-hold vault (deposit CETES, hold CETES,
withdraw CETES) cannot work: value-accruing tokens don't grow their *balance*, so the vault's
balance-delta fee machinery sees zero yield and can't tax anything. Realizing accrual as
*base-asset* balance growth is what makes this a shippable product.

Docs for this strategy live in [`docs/strategies/rwa-yield-sharing/`](../../docs/strategies/rwa-yield-sharing/):
architecture, economic design, and threat model.

## Interface

The vault interacts only through `DeFindexStrategyTrait`:

| Method | Notes |
|---|---|
| `asset()` | The underlying (USDC). |
| `deposit(amount, from)` | Accepts the whole `amount` into idle USDC and mints at par against pre-deposit NAV; refused outside `Live` pricing while RWA is held. If `auto_invest` is on, buys one best-effort tranche of at most the deposit and `max_swap`, never the idle buffer. |
| `withdraw(amount, from, to)` | Idle USDC first, then one pool sale of at most the shortfall (≤ `max_swap`). Never allowlist-gated, never pausable. |
| `balance(from)` | Total: never reverts on an oracle or pool failure, writes nothing. |
| `harvest(from, data)` | Keeper-only telemetry no-op; `data` is ignored. |

Outside the trait, two roles:

- **Keeper** (bot): `invest` / `divest` (pool tranches), `invest_otc` / `divest_otc` (OTC fills at
  a price inside a timelocked band around the rate, ships 0), `pause_deposits`, and `freeze_price`
  (freeze only, for 24 h, once: a second keeper freeze is refused until the admin unfreezes or
  calls `set_keeper`).
- **Admin** (multisig): allowlists, `set_keeper`, two-step admin transfer, `set_caps` /
  `set_pricing_params` / `set_otc_max_deviation_bps` (lower only), `pause_deposits`, `freeze_price` (freeze
  indefinitely or unfreeze; an unfreeze also re-arms the keeper's one freeze), `set_auto_invest`, `reanchor_to_oracle` (books a confirmed NAV drop fast: lowers the anchor toward
  the oracle rate, at most 5% per call and one call per hour, never raises it), and three 72 h-timelocked proposals: the oracle
  set, an OTC counterparty, and any raise of the price knobs.

No method sends tokens to an address the caller chooses, and the pool can never be changed.

## Pricing

Held RWA is valued at its **oracle NAV, never at the pool**: the lowest live oracle (1-3 SEP-40
or RedStone sources; every preset uses one RedStone adapter), clamped to `max_spread_bps` around
`LastValidPrice`, which moves at most once per hour. A wrong oracle can therefore move NAV at most
`max_spread_bps` (10 bps at the presets) per hour, and the admin or keeper can freeze the rate at
the anchor at once. With no live oracle the rate is the anchor. The pool only executes swaps, each
bounded by `slippage_bps` around the oracle rate, so buying from a pool under NAV is accretive and
selling into one under NAV by more than `slippage_bps` reverts. The presets ship `slippage_bps` 0,
so a withdrawal's shortfall sells into the pool only at NAV or better and never costs the holders
who stay; idle is kept near 10% and refilled OTC at NAV. Why the
pool is out: the USDY and deJTRSY pools sit 120-150 bps under NAV for weeks, and Reflector's
Stellar feed is itself a pool price.

## Deployment

Constructor: `__constructor(asset: Address, init_args: Vec<Val>)`, where `asset` is USDC and
`init_args` is **9 values, in this order**:

| # | Name | Type |
|---|---|---|
| 0 | `rwa_asset` | `Address` |
| 1 | `admin` | `Address` (multisig) |
| 2 | `keeper` | `Address` (bot) |
| 3 | `oracles` | `Vec<OracleSource>` — 1..=3, each `Sep40(feed, asset)` or `RedStone(adapter)` with its own `ttl_seconds` and `decimals` |
| 4 | `venue` | `Venue` — `Aqua(pool, rwa_idx, usdc_idx)` or `Sushi(pool, rwa_is_token0)`, fixed for life |
| 5 | `max_swap` | `i128`, USDC per swap |
| 6 | `max_invested` | `i128`, USDC value of held RWA |
| 7 | `max_spread_bps` | `u32`, 10..=500 |
| 8 | `slippage_bps` | `u32`, ≤ 500 |

Construction **panics** on bad input, so a failed deploy persists nothing. It reads `decimals()`
on both tokens and every oracle, and the pool's token set, live, so it only works against deployed contracts (testnet or mainnet). The
contract is **not upgradable**.

`scripts/deploy.sh` has `ustry`, `usdy` and `dejtrsy` presets (RedStone-only, `max_spread_bps`
10, `slippage_bps` 0) plus `custom`. The other scripts in `scripts/` wrap each operation, resolving the instance
from `<network>.contracts.json` through `DEPLOYMENT=<ustry|usdy|dejtrsy>`.

## Build & test

```bash
# Build (wasm32v1-none) and optimize
stellar contract build
stellar contract optimize --wasm target/wasm32v1-none/release/stablebond_wrapper_strategy.wasm

# Tests: mock unit suite + real-WASM integration suite (Aqua, Reflector-shaped SEP-40, Sushi, RedStone)
cargo test -p stablebond_wrapper_strategy
```
