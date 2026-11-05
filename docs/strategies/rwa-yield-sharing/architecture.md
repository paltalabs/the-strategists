# RWA Yield-Sharing Strategy: architecture

This document describes how the RWA yield-sharing strategy is built: its components, roles,
flows, share accounting, pricing, admin surface, and where revenue sharing happens. The economic
reasoning behind the design is in [`economic-design.md`](./economic-design.md); the security
analysis is in [`threat-model.md`](./threat-model.md).

Source: `contracts/stablebond-wrapper-strategy/`. All `file:line` references are relative to the
repository root. The contract is not upgradable; every behavior below is fixed at deployment.

## 1. Overview

The strategy is a DeFindex vault strategy: it implements `DeFindexStrategyTrait` and is called
only by a DeFindex vault. The vault deposits **USDC**. The strategy keeps part of it idle as a
withdrawal buffer and converts the rest into a **value-accruing RWA token** (a tokenized
treasury or stable bond), bought either from an allowlisted OTC counterparty at the oracle NAV or
in capped tranches on one fixed AMM pool. Withdrawals are paid in USDC, from idle first.

Because the RWA's price rises while its balance stays constant, the strategy values its holdings
at the issuer's NAV (read from an oracle) and reports `idle USDC + held RWA x NAV` to the vault.
That reported balance grows as NAV accrues, which is the only signal the DeFindex vault needs to
book yield, charge its performance fee and split it between participants (section 8).

```
            depositors (users of a partner app)
                     | USDC in / USDC out, dfTokens
                     v
           +--------------------+      fee split on reported gains:
           |   DeFindex vault   | ---> vault fee receiver (partner app)
           +--------------------+ ---> protocol receiver (infrastructure provider)
                     | deposit / withdraw / balance (DeFindexStrategyTrait)
                     v
           +--------------------+  reads   +-----------------------------+
           |  this strategy     | <------- | NAV oracles (RedStone,      |
           |  idle USDC + RWA   |          | SEP-40), 1 to 3 sources     |
           +--------------------+          +-----------------------------+
              |             |
     swaps    |             |  atomic two-leg fills at the bounded rate
              v             v
   +----------------+   +-------------------------------+
   | one fixed pool |   | allowlisted OTC counterparty  |
   | Aqua or Sushi  |   | (mints/redeems with issuer)   |
   +----------------+   +-------------------------------+
```

## 2. Components

| Component | What it is | How the strategy uses it | Reference |
|---|---|---|---|
| DeFindex vault | The share-token vault that holds user deposits and allocates them to strategies | The only depositor. Calls `deposit`, `withdraw`, `balance`, `harvest` | `contracts/stablebond-wrapper-strategy/src/lib.rs:92-304` |
| Strategy | This contract | Holds idle USDC and the RWA, keeps per-vault shares, prices NAV | `contracts/stablebond-wrapper-strategy/src/lib.rs:89-90` |
| Underlying asset | USDC (`Config.asset`) | What the vault deposits and withdraws | `contracts/stablebond-wrapper-strategy/src/storage.rs:47-48` |
| RWA token | A value-accruing SEP-41 token (`Config.rwa_asset`), e.g. USTRY, USDY, deJTRSY, CETES | Bought, held and sold; valued at oracle NAV | `contracts/stablebond-wrapper-strategy/src/storage.rs:49-50` |
| Venue | One Aqua pool or one Sushi concentrated pool, called directly with no router | Executes swaps only; never a pricing input | `contracts/stablebond-wrapper-strategy/src/storage.rs:33-41`, `contracts/stablebond-wrapper-strategy/src/venue.rs:1-98` |
| Oracles | 1 to `MAX_ORACLES` (3) sources, each a SEP-40 feed (Reflector-shaped) or a RedStone per-asset adapter | Source of the NAV rate | `contracts/stablebond-wrapper-strategy/src/storage.rs:10-31`, `contracts/stablebond-wrapper-strategy/src/oracles.rs:61-83` |
| OTC counterparty | An allowlisted address, typically a KYC'd entity that can mint and redeem with the issuer | Atomic USDC/RWA fills at a price inside a band around the oracle rate | `contracts/stablebond-wrapper-strategy/src/otc.rs:1-128` |

The configuration that names the tokens, their cached decimals and the venue is immutable
(`Config`, `contracts/stablebond-wrapper-strategy/src/storage.rs:43-57`). The venue has no
setter: moving to another pool means deploying a new strategy. The constructor checks every
dependency live before storing anything: token decimals (bounded by `MAX_DECIMALS` = 18), each
oracle's reachability and decimals, and that the pool actually holds the RWA and USDC at the
configured positions (`contracts/stablebond-wrapper-strategy/src/lib.rs:109-155`,
`contracts/stablebond-wrapper-strategy/src/oracles.rs:33-50`,
`contracts/stablebond-wrapper-strategy/src/venue.rs:16-50`). A failed check panics, so a bad
deployment persists nothing.

## 3. Roles

| Role | Who | Powers | Cannot |
|---|---|---|---|
| Depositor | A DeFindex vault, allowlisted by exact address or by wasm code hash (`contracts/stablebond-wrapper-strategy/src/lib.rs:61-69`) | `deposit`, `withdraw` its own shares | Withdraw another vault's shares |
| Keeper | Operations bot (`Params.keeper`) | `invest` / `divest` pool tranches, `invest_otc` / `divest_otc`, `harvest`, `pause_deposits`, one 24 h `freeze_price` | Name a token recipient other than an allowlisted counterparty; unfreeze; change any parameter |
| Admin | Multisig (`Params.admin`) | Allowlists, `set_keeper`, lower-only knob setters, indefinite freeze and unfreeze, `reanchor_to_oracle`, three timelocked proposals, two-step admin transfer | Change the venue; set a price; raise a price knob without a 72 h public wait; pay an arbitrary address |
| Vault manager | DeFindex vault role (outside this contract) | Pause the strategy in the vault, rescue, rebalance | Nothing in this contract: it acts through the vault |
| Depositors (end users) | Users of the partner application | Deposit into and withdraw from the vault | Interact with the strategy directly |

Role storage: `contracts/stablebond-wrapper-strategy/src/storage.rs:84-123`. Every keeper entry
point calls `params.keeper.require_auth()` before any state write
(`contracts/stablebond-wrapper-strategy/src/lib.rs:214`, `:408`, `:438`, `:469`, `:483`); every
admin entry point goes through `require_admin`
(`contracts/stablebond-wrapper-strategy/src/lib.rs:72-74`).

## 4. Flows

### 4.1 Deposit (`deposit(amount, from)`)

`contracts/stablebond-wrapper-strategy/src/lib.rs:166-206`

1. `from.require_auth()` and the depositor allowlist check (`:169-172`).
2. RWA balance tripwire: compare the live RWA balance with the balance accounted flows imply;
   alert on a shortfall, never revert (`:175`, `:78-87`).
3. Refuse when `paused` (`:176-178`).
4. Price NAV with the guarded pricing path (`:179`). If the strategy holds RWA and pricing is not
   `Live`, refuse (`:180-182`): minting against a stale anchor would misprice the new shares.
5. Pull the whole `amount` of USDC into idle (`:185`) and mint shares against the
   **pre-deposit** NAV, rounding down (`:186-196`). The deposit enters at par: one USDC adds one
   USDC of NAV.
6. Optionally buy one tranche (`:200`): only when `auto_invest` is on (it ships off), deposits are
   not paused, and pricing allows a buy. The tranche is at most the deposit, `max_swap` and the
   `max_invested` room, never the existing idle buffer, and is skipped silently if the pool's
   quote misses the oracle-implied minimum
   (`contracts/stablebond-wrapper-strategy/src/flows.rs:55-71`).
7. Return the vault's balance computed from the pre-swap NAV plus `amount` (`:202-205`), so a
   deposit never reports its own swap's price impact as a gain or loss to the vault.

### 4.2 Invest (keeper)

The keeper moves idle USDC into the RWA through one of two paths:

| Path | Entry point | Bounds | Reference |
|---|---|---|---|
| Pool tranche | `invest(usdc_amount)` | Keeper auth; not paused; `usdc_amount <= max_swap`, `<= idle`, `<= max_invested` room; pricing must pass `can_buy`; `min_out` = oracle-implied RWA minus `slippage_bps` | `contracts/stablebond-wrapper-strategy/src/lib.rs:405-429`, `contracts/stablebond-wrapper-strategy/src/flows.rs:14-17` |
| OTC fill | `invest_otc(counterparty, usdc_amount, price)` | Keeper auth; counterparty allowlisted; not paused; `can_buy`; `price` within `otc_max_deviation_bps` of the rate; `<= idle`, `<= max_invested` room | `contracts/stablebond-wrapper-strategy/src/lib.rs:466-475`, `contracts/stablebond-wrapper-strategy/src/otc.rs:46-87` |

`can_buy` requires `Live` pricing, a positive rate, and that the raw oracle reading is not below
the clamped rate (`contracts/stablebond-wrapper-strategy/src/pricing.rs:79-81`). When the clamp
is holding NAV above what the oracle says, buying at that rate would overpay, so every buy path
refuses.

Because the pool's `min_out` is derived from the oracle rate and not from the pool, a pool that
trades under NAV fills above `min_out` (an accretive buy) and a pool pushed above NAV reverts.

The OTC path is atomic: the strategy pulls the counterparty's RWA with `transfer_from` and pays
USDC with `transfer` in the same invocation, so either both legs land or neither does
(`contracts/stablebond-wrapper-strategy/src/otc.rs:74-75`). It is not bounded by `max_swap`
because it has no pool impact.

### 4.3 Withdraw (`withdraw(amount, from, to)`)

`contracts/stablebond-wrapper-strategy/src/lib.rs:234-290`

1. `from.require_auth()` (`:237`). No allowlist check, no pause check and no pricing-mode gate:
   a vault can always exit.
2. Tripwire and guarded pricing (`:240-241`).
3. Refuse if `amount` exceeds the vault's share value (`:245-248`). Shares to burn are computed
   rounding **up**, and clamped to the vault's holding on a full exit (`:249-252`).
4. Serve from idle first (`:254`). For any shortfall:
   - refuse if the shortfall exceeds `max_swap` or the rate is zero (`:258-260`);
   - sell at most the oracle-implied RWA quantity plus `slippage_bps`, both rounded up
     (`contracts/stablebond-wrapper-strategy/src/flows.rs:25-28`), clamped to what is held
     (`:261-267`);
   - require at least the shortfall in USDC back (`:268`), so a short fill reverts instead of
     short-paying.
5. Transfer exactly `amount` USDC to `to` (`:273`), the address the vault itself supplied.
6. Update shares and the expected RWA balance (`:275-279`); return the remaining balance from the
   pre-trade NAV minus `amount` (`:282-289`).

With the shipped `slippage_bps` of 0 a shortfall sells only at NAV or better. If the pool is
under NAV, the shortfall sale reverts and the withdrawal waits for the keeper to refill idle
(section 4.4). This is a liveness cost by design: it keeps exits from transferring value away
from the holders who stay (see [`economic-design.md`](./economic-design.md), section 6).

### 4.4 Divest (keeper)

| Path | Entry point | Bounds | Reference |
|---|---|---|---|
| Pool tranche | `divest(usdc_target)` | Keeper auth; `usdc_target <= max_swap`; any mode with a positive rate; gives up at most oracle-implied RWA plus `slippage_bps`; `min_out` = target | `contracts/stablebond-wrapper-strategy/src/lib.rs:435-460` |
| OTC fill | `divest_otc(counterparty, usdc_amount, price)` | Keeper auth; counterparty allowlisted; any mode with a positive rate, `Frozen` included; `price` within the band; refuses rather than clamps if the RWA leg exceeds holdings | `contracts/stablebond-wrapper-strategy/src/lib.rs:480-489`, `contracts/stablebond-wrapper-strategy/src/otc.rs:93-128` |

Proceeds of both stay in the strategy as idle USDC. Neither path is pausable, so the position
can always be unwound.

### 4.5 Harvest (`harvest(from, data)`)

There is nothing to claim: yield is price appreciation. `harvest` is a keeper-only telemetry
call (`contracts/stablebond-wrapper-strategy/src/lib.rs:211-228`). It runs the tripwire and
guarded pricing (which may advance the anchor, section 6), then emits the DeFindex
`HarvestEvent` with amount 0 and the current price per share in `Live` mode, or a
`SpreadAlertEvent` otherwise. It moves no funds.

### 4.6 Balance (`balance(from)`)

`contracts/stablebond-wrapper-strategy/src/lib.rs:294-303`

`balance` is **total**: it never writes storage, never bumps a TTL, and never reverts because an
oracle or the pool failed. Every oracle read is a `try_` call that maps failure to "not live"
(`contracts/stablebond-wrapper-strategy/src/oracles.rs:61-83`), rates above `MAX_RATE` are
treated as not live so later arithmetic cannot overflow
(`contracts/stablebond-wrapper-strategy/src/constants.rs:58-64`), and the pool is never read. This
matters because the DeFindex vault reads every strategy's `balance` on each vault deposit and
withdrawal; a strategy whose `balance` reverted would block the whole vault.

## 5. Share accounting

The strategy keeps its own shares per depositing vault (`DataKey::VaultPos(vault)` in persistent
storage, total in `DataKey::TotalShares`;
`contracts/stablebond-wrapper-strategy/src/storage.rs:127-156`). One strategy can therefore
serve several vaults, each with its own claim.

Conversions use an ERC-4626 style virtual offset
(`contracts/stablebond-wrapper-strategy/src/shares.rs:1-68`):

```
shares_minted = assets_in * (total_shares + VIRTUAL_SHARES) / (nav_before + 1)     floor
shares_burned = amount    * (total_shares + VIRTUAL_SHARES) / (nav + 1)            ceil
vault_value   = shares    * (nav + 1) / (total_shares + VIRTUAL_SHARES)            floor
```

`VIRTUAL_SHARES` is 1,000,000 (`contracts/stablebond-wrapper-strategy/src/constants.rs:6`). The
offset makes the classic first-depositor inflation attack (donate assets to skew the first share
price) uneconomical without burning real shares.

Rounding always favors the strategy, that is, the holders who stay: floor on mint, ceil on burn,
floor on the reported value. A full exit clamps the burn to the vault's shares and removes the
storage entry (`contracts/stablebond-wrapper-strategy/src/lib.rs:250-252`,
`contracts/stablebond-wrapper-strategy/src/storage.rs:215-223`).

NAV itself is:

```
NAV = idle USDC + value_of(held RWA, rate)
value_of(qty, rate) = qty * rate / (10^rwa_decimals * 10^12)                         floor
```

(`contracts/stablebond-wrapper-strategy/src/pricing.rs:110-119`,
`contracts/stablebond-wrapper-strategy/src/math.rs:50-56`). `rate` is USDC base units per whole
RWA scaled by `SCALAR_12` (10^12), derived from an oracle price as
`price * 10^asset_decimals * 10^12 / 10^price_decimals`
(`contracts/stablebond-wrapper-strategy/src/math.rs:35-46`). Every intermediate that could
exceed `i128` goes through `I256`, so 18-decimal RWAs at multi-million positions do not overflow
(`contracts/stablebond-wrapper-strategy/src/math.rs:1-2`).

## 6. Pricing

Pricing is the security core of the strategy
(`contracts/stablebond-wrapper-strategy/src/pricing.rs:1-201`). Inputs are the live oracle rates
`O` and the anchor `L` (`LastValidPrice`). **The pool is not an input**: it only executes
swaps, each bounded by `slippage_bps` around the oracle rate.

An oracle source is live only if the call succeeds, the price is positive, its timestamp is within
the source's `ttl_seconds` of the ledger clock (at most `MAX_TTL_SECONDS` = 86,400 s), and the
normalized rate is in `(0, MAX_RATE]` (`contracts/stablebond-wrapper-strategy/src/oracles.rs:61-83`).
RedStone timestamps are milliseconds and are converted (`:74`).

### 6.1 Pricing modes

| Mode | Condition | Rate | What it allows | Reference |
|---|---|---|---|---|
| `Live` | Not frozen, at least one live oracle | `min(O)` clamped to `[L*(1-s), L*(1+s)]`, `s = max_spread_bps`; the raw minimum if no anchor exists yet | Everything. The only mode that writes the anchor, buys RWA, or mints against held RWA | `contracts/stablebond-wrapper-strategy/src/pricing.rs:95-101` |
| `OracleDown` | Not frozen, no live oracle | `L`, or 0 with no anchor | Withdrawals, `divest`, `divest_otc` at `L`. Deposits against held RWA and all buys refuse | `contracts/stablebond-wrapper-strategy/src/pricing.rs:102-105` |
| `Frozen` | Ledger time before `Params.frozen_until` | `L`, or 0 with no anchor, whatever the oracles say | Same as `OracleDown` | `contracts/stablebond-wrapper-strategy/src/pricing.rs:89-94` |

The minimum of the live sources wins, so with several sources the most conservative reading is
used.

### 6.2 The anchor clamp

The anchor `L` is written only in `Live`, only from the clamped rate, and at most once per
`ANCHOR_WINDOW_SECONDS` (3,600 s) (`contracts/stablebond-wrapper-strategy/src/pricing.rs:126-152`,
`contracts/stablebond-wrapper-strategy/src/constants.rs:25`). Each write therefore moves `L` at
most one spread. Consequence: **no oracle, however wrong, can move NAV by more than
`max_spread_bps` per hour** in either direction. `max_spread_bps` is bounded to
`[MIN_SPREAD_BPS, MAX_SPREAD_BPS]` = [10, 500] bps
(`contracts/stablebond-wrapper-strategy/src/constants.rs:14-15`); the shipped deployment presets
use 10 (`contracts/stablebond-wrapper-strategy/scripts/deploy.sh:103`, `:114`, `:122`).

A real NAV feed moves far less than the clamp per hour, so in normal operation the clamp never
binds. When it does, the guarded pricing path emits `PriceJumpEvent` whenever the raw rate is more
than one spread from the anchor (`contracts/stablebond-wrapper-strategy/src/pricing.rs:133-135`)
and `PricingModeEvent` whenever the mode is not `Live` (`:142-150`). `get_pricing_state` exposes
both the clamped `rate` and the unclamped `raw_rate` so the keeper can detect a binding clamp
(`contracts/stablebond-wrapper-strategy/src/lib.rs:492-497`).

The first anchor is the exception: with no `LastValidPrice`, the first `Live` state-changing call
writes the raw oracle rate unclamped (`contracts/stablebond-wrapper-strategy/src/pricing.rs:140`).
The operator checks the oracle against the issuer's published NAV before the first deposit.

### 6.3 Freeze and reanchor

- `freeze_price(caller, frozen)` holds the rate at the anchor (`Frozen` mode)
  (`contracts/stablebond-wrapper-strategy/src/lib.rs:709-728`). The admin freezes indefinitely
  and is the only role that unfreezes. The keeper may freeze once, for `KEEPER_FREEZE_SECONDS`
  (86,400 s), never shortening a longer freeze; its freeze is re-armed only by an admin unfreeze
  or `set_keeper`. A freeze can only hold the last agreed price, never set one.
- `reanchor_to_oracle(expected_rate)` lets the admin book a confirmed NAV **drop** faster than one
  spread per hour (`contracts/stablebond-wrapper-strategy/src/lib.rs:741-749`,
  `contracts/stablebond-wrapper-strategy/src/pricing.rs:170-201`). It requires: not frozen, no
  reanchor in the last `ANCHOR_WINDOW_SECONDS`, every configured oracle live and within
  `max_spread_bps` of the multisig's `expected_rate`, and their minimum strictly below the anchor.
  It lowers the anchor to that minimum or by at most `MAX_REANCHOR_STEP_BPS` (500 bps), whichever
  is higher. It can never raise NAV.

## 7. Admin surface and timelocks

The admin can **narrow** any price bound immediately and can **widen** one only through a public
proposal executable `TIMELOCK_SECONDS` (259,200 s, 72 h) later
(`contracts/stablebond-wrapper-strategy/src/constants.rs:20`,
`contracts/stablebond-wrapper-strategy/src/timelock.rs:1-169`). Removing trust is always
immediate.

| Change | Immediate | Timelocked (72 h) | Reference |
|---|---|---|---|
| `max_swap` | Lower only (`set_caps`) | Raise via `propose_params` / `execute_params` | `contracts/stablebond-wrapper-strategy/src/lib.rs:659-676`, `:527-545` |
| `max_invested` | Either way (`set_caps`); it caps exposure, not price | n/a | `contracts/stablebond-wrapper-strategy/src/lib.rs:659-676` |
| `max_spread_bps`, `slippage_bps` | Lower only (`set_pricing_params`), within the constructor's bounds | Raise via `propose_params` | `contracts/stablebond-wrapper-strategy/src/lib.rs:681-698` |
| `otc_max_deviation_bps` (OTC band, ships 0, cap `MAX_OTC_DEVIATION_BPS` = 500) | Narrow only (`set_otc_max_deviation_bps`) | Widen via `propose_params` | `contracts/stablebond-wrapper-strategy/src/lib.rs:556-571` |
| Oracle set | Cancel a pending proposal | `propose_oracles` / `execute_oracles`; execution also needs one proposed source live | `contracts/stablebond-wrapper-strategy/src/lib.rs:501-522`, `contracts/stablebond-wrapper-strategy/src/oracles.rs:92-99` |
| OTC counterparty | Remove (`remove_otc_counterparty`) | Add via `propose_otc_counterparty` / `execute_otc_counterparty` | `contracts/stablebond-wrapper-strategy/src/lib.rs:575-594` |
| Keeper | `set_keeper` (also re-arms the keeper's freeze) | n/a | `contracts/stablebond-wrapper-strategy/src/lib.rs:643-653` |
| Admin | n/a | Two-step: `transfer_admin`, then `accept_admin` signed by the new admin | `contracts/stablebond-wrapper-strategy/src/lib.rs:616-638` |
| Deposit allowlist | Add or remove address / wasm hash | n/a | `contracts/stablebond-wrapper-strategy/src/lib.rs:330-375` |
| Pause deposits and buys | `pause_deposits` (admin or keeper); never affects `withdraw` or `divest` | n/a | `contracts/stablebond-wrapper-strategy/src/lib.rs:316-327` |
| `auto_invest` | `set_auto_invest` (ships off) | n/a | `contracts/stablebond-wrapper-strategy/src/lib.rs:390-398` |
| Venue | No setter | No setter | `contracts/stablebond-wrapper-strategy/src/storage.rs:55-56` |

The contract can pay exactly three kinds of address: the `to` the vault supplies on `withdraw`,
the fixed pool during a swap, and an OTC counterparty that was announced 72 h before it could
trade. No method takes a price or a destination chosen by the admin.

## 8. Where revenue sharing happens

Revenue sharing is **not** implemented in the strategy. It happens in the DeFindex vault, which
already has a configurable two-receiver fee split; the strategy's job is to make the yield
visible to it.

1. **Measurement (strategy).** `balance(vault)` returns the vault's share of
   `idle + held RWA x NAV`. As NAV accrues, this number grows with no change in token balances.
2. **Booking (vault).** On deposits, withdrawals and fee operations the vault refreshes a
   per-strategy report: the change in the strategy's reported balance since the last report, net
   of the vault's own deposits into and withdrawals from the strategy, accumulates as
   `gains_or_losses`.
3. **Fee lock (vault).** `lock_fee` converts positive `gains_or_losses` into `locked_fee =
   gains * vault_fee / 10,000` and resets the gain counter. Losses stay booked and must be earned
   back before a new fee accrues. Locked fees are excluded from the assets that back the vault's
   shares, so share prices are always net of fees owed.
4. **Distribution (vault).** `distribute_fees` withdraws `locked_fee` USDC from the strategy
   (through this strategy's `withdraw`, served from idle first), pays
   `locked_fee * defindex_protocol_rate / 10,000` to the DeFindex protocol receiver and the rest to
   the vault fee receiver.
5. **Depositors** keep everything else, distributed pro rata through the vault's share price.

| Participant | DeFindex vault mechanism | Configured by |
|---|---|---|
| User (depositor) | Gross yield minus `vault_fee`, through the vault share price | n/a |
| Partner application | Vault fee receiver: `vault_fee x (1 - defindex_protocol_rate)` of gains | `vault_fee` per vault; receiver set by the vault manager |
| Infrastructure provider | DeFindex protocol receiver: `vault_fee x defindex_protocol_rate` of gains | Protocol rate set on the factory (at most 9,000 bps of the fee), stamped into each vault at creation |

The split is configurable per vault, so one strategy deployment can serve several partners at
different fee levels. A worked example is in
[`economic-design.md`](./economic-design.md), section 5.

Attribution to individual deposits happens at two levels, both pro rata by shares:

- **Strategy level:** each depositing vault holds strategy shares minted at the NAV at the moment
  of its deposit (section 5).
- **Vault level:** each user holds vault shares (dfTokens). Because the vault locks fees before
  minting or burning shares, a user who enters after a gain does not pay the fee on that gain, and
  a user who exits takes only their share of gains net of fees accrued while they held.

## 9. Interface reference

### 9.1 Constructor

`__constructor(asset: Address, init_args: Vec<Val>)`, `asset` = USDC
(`contracts/stablebond-wrapper-strategy/src/lib.rs:94-155`).

| # | Name | Type | Bound |
|---|---|---|---|
| 0 | `rwa_asset` | `Address` | `!= asset`; decimals `<= MAX_DECIMALS` (18) |
| 1 | `admin` | `Address` | multisig expected |
| 2 | `keeper` | `Address` | bot |
| 3 | `oracles` | `Vec<OracleSource>` | 1..=`MAX_ORACLES` (3); each `ttl_seconds` in `(0, MAX_TTL_SECONDS]`; `decimals <= 18` and equal to the live `decimals()` |
| 4 | `venue` | `Venue` | `Aqua(pool, rwa_idx, usdc_idx)` or `Sushi(pool, rwa_is_token0)`; token order checked live |
| 5 | `max_swap` | `i128` | `> 0`, USDC base units per swap |
| 6 | `max_invested` | `i128` | `> 0`, USDC value cap on held RWA |
| 7 | `max_spread_bps` | `u32` | `MIN_SPREAD_BPS..=MAX_SPREAD_BPS` (10..=500) |
| 8 | `slippage_bps` | `u32` | `<= MAX_SLIPPAGE_BPS` (500) |

Initial state: `paused = false` (the empty allowlist already blocks deposits), `auto_invest =
false`, not frozen, OTC band 0, total shares 0 (`:136-154`).

### 9.2 Public methods

| Method | Caller | Purpose |
|---|---|---|
| `asset()` | anyone | USDC address |
| `deposit(amount, from)` | allowlisted vault | Section 4.1 |
| `withdraw(amount, from, to)` | share-holding vault | Section 4.3 |
| `balance(from)` | anyone | Section 4.6 |
| `harvest(from, data)` | keeper | Section 4.5 |
| `invest(usdc_amount)` / `divest(usdc_target)` | keeper | Pool tranches |
| `invest_otc(counterparty, usdc_amount, price)` / `divest_otc(...)` | keeper | OTC fills |
| `pause_deposits(caller, paused)` | admin or keeper | Pause deposits and buys |
| `freeze_price(caller, frozen)` | admin; keeper (freeze once) | Section 6.3 |
| `reanchor_to_oracle(expected_rate)` | admin | Section 6.3 |
| `set_caps`, `set_pricing_params`, `set_otc_max_deviation_bps` | admin | Lower-only knob setters |
| `propose_params` / `execute_params` / `cancel_params` | admin | Timelocked knob changes |
| `propose_oracles` / `execute_oracles` / `cancel_oracles` | admin | Timelocked oracle set |
| `propose_otc_counterparty` / `execute_otc_counterparty` / `remove_otc_counterparty` | admin | Timelocked counterparty allowlist |
| `add_depositor`, `remove_depositor`, `add_allowed_hash`, `remove_allowed_hash` | admin | Deposit allowlist |
| `set_keeper`, `transfer_admin`, `accept_admin`, `set_auto_invest` | admin (pending admin for `accept_admin`) | Roles and flags |
| `get_config`, `get_params`, `get_pricing_state`, `get_pending_oracles`, `get_pending_params`, `get_pending_otc_counterparty`, `is_depositor`, `is_allowed_hash`, `is_otc_counterparty` | anyone | Views |

### 9.3 Events

Besides the DeFindex trait events (deposit, withdraw, harvest), the strategy emits one typed
event per state change, each topic-tagged with the strategy name
(`contracts/stablebond-wrapper-strategy/src/events.rs`):

| Group | Events |
|---|---|
| Trading | `InvestEvent`, `DivestEvent`, `OtcInvestEvent` (with `price` and `rate`), `OtcDivestEvent` |
| Pricing alerts | `PricingModeEvent`, `PriceJumpEvent`, `SpreadAlertEvent`, `PriceFrozenEvent`, `ReanchorEvent` |
| Asset integrity | `RwaBalanceAnomalyEvent` (live RWA below accounted balance) |
| Governance notices | `OraclesProposedEvent`, `ParamsProposedEvent`, `OtcCounterpartyProposedEvent`, `AdminTransferStartedEvent` |
| Governance changes | `OraclesSetEvent`, `OraclesProposalCancelledEvent`, `ParamsUpdatedEvent`, `ParamsProposalCancelledEvent`, `OtcCounterpartySetEvent`, `AdminTransferredEvent`, `SetKeeperEvent`, `SetDepositorEvent`, `SetAllowedHashEvent`, `PauseEvent`, `AutoInvestSetEvent` |

### 9.4 Storage and TTL

`Config`, `Params`, total shares, the anchor and pending proposals live in instance storage,
bumped to 30 days on every entry point that touches storage except `balance`. Per-vault shares,
allowlists and pending counterparties live in persistent storage with a 120-day bump
(`contracts/stablebond-wrapper-strategy/src/storage.rs:158-176`). The keeper's loop also extends
the TTL of the oracle and venue contracts the strategy depends on
(`contracts/stablebond-wrapper-strategy/scripts/deploy.sh:244-246`).
