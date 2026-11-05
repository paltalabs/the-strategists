# RWA Yield-Sharing Strategy: economic design

This document covers the research behind the strategy: which yield-bearing assets it supports,
how their yield accrues, how accrued profit is measured reliably, how that profit is shared
between the user, the partner application and the infrastructure provider, and what liquidity
constraints shape the design. The contract itself is described in
[`architecture.md`](./architecture.md); risks are in [`threat-model.md`](./threat-model.md).

Contract references are `file:line` paths relative to the repository root. Market figures in
this document (pool discounts, depth, price impact) are point-in-time measurements taken on
Stellar mainnet during the research; they explain design choices and are not constants of the
contract.

## 1. Compatible yield-bearing assets

The target assets are tokenized short-duration government debt and treasury funds that live on
Stellar as SEP-41 tokens (classic assets with a Stellar Asset Contract, or custom Soroban tokens).
All of them share one property that drives the whole design: **they are value-accruing**. The
holder's balance never changes; the token's price rises as the underlying bonds accrue interest.

| Asset | Issuer | Underlying | Denomination of NAV | NAV source on Stellar | Main venue on Stellar | Notes |
|---|---|---|---|---|---|---|
| USTRY | Etherfuse | US Treasury bills | USD | RedStone feed of the issuer NAV | Aqua USTRY/USDC pool | An issuer-side market maker keeps the pool close to NAV |
| CETES | Etherfuse | Mexican federal treasury certificates | MXN | RedStone feed | Aqua CETES/USDC pool | USD value also moves with MXN/USD, so the USD NAV is not monotonic |
| USDY | Ondo | US Treasuries and bank deposits | USD | RedStone "fundamental" feed of Ondo's NAV | Aqua USDY/USDC pool | Issuer retains revocation and clawback flags on the asset |
| deJTRSY | Centrifuge | Tokenized treasury fund | USD | RedStone "fundamental" feed of the fund's token price | Sushi deJTRSY/USDC concentrated pool | Custom 18-decimal token with an issuer admin surface |

Other Etherfuse stablebonds (TESOURO, EUROB, KTB) have the same shape and are supported by the
same contract, but at the time of research their Stellar supply and pool depth were too small to
deploy against.

What makes an asset compatible with this strategy:

1. It is a SEP-41 token a contract can hold (no per-holder authorization that a contract cannot
   satisfy).
2. A NAV feed exists that reports the **issuer's** price, not a market price, through a SEP-40
   or RedStone adapter interface (`contracts/stablebond-wrapper-strategy/src/storage.rs:10-31`).
3. A USDC pair exists on one Aqua or Sushi pool for incremental flows, and ideally a KYC'd
   counterparty that mints and redeems with the issuer for size (section 6).
4. Its token and feed decimals are at most `MAX_DECIMALS` = 18
   (`contracts/stablebond-wrapper-strategy/src/constants.rs:53`).

The shipped deployment presets cover USTRY, USDY and deJTRSY, each priced by one RedStone
adapter (`contracts/stablebond-wrapper-strategy/scripts/deploy.sh:94-123`).

## 2. How the yield accrues

Each issuer computes a net asset value per token off-chain from the bonds or fund it holds, and
the token's price follows that NAV. On Stellar there is no rebasing and no claimable reward:

- A contract holding 100 tokens today holds 100 tokens next year.
- The tokens' worth in USD rises with the NAV, roughly at the bond yield.
- The yield is only realized when the tokens are sold or redeemed.

The RedStone feeds publish this NAV on Stellar on a heartbeat (observed at 12 h for the push
feeds the presets use), and a NAV feed moves in small steps: the largest single NAV write we
observed over a 90-day window was 4 bps. That is why the contract can bound how far a feed may
move NAV per hour without ever binding on a healthy feed (section 4.2).

## 3. Why USDC in, USDC out (research finding)

The obvious product, a vault that takes the RWA token, holds it, and returns it, **cannot share
yield**. The DeFindex vault books a strategy's profit as growth in the strategy's reported
balance of the vault's underlying asset. If the underlying is the RWA itself, that balance never
grows: the vault's gain counter stays at zero, there is nothing to lock a fee on, and no revenue
share is possible. The yield is real, but invisible to balance-delta fee accounting.

The fix is to change the unit of account. The vault's underlying is **USDC**; the strategy holds
the RWA and reports its holdings **in USDC at NAV**:

```
reported balance = idle USDC + held RWA x NAV(USDC per RWA)
```

As NAV accrues, the reported USDC balance grows with no change in token balances. The vault sees
that growth as gains, charges its fee on it, and pays the fee out in USDC withdrawn from the
strategy. Users deposit and withdraw USDC, so their profit is also measured in the same asset the
fees are paid in.

This is the core research result: **holding a value-accruing asset and settling in a stable base
asset turns price accrual into measurable base-asset growth**, which is what the existing vault
fee machinery needs to attribute and share yield. It also lets one vault combine several RWAs
(section 7), since they all report in the same unit.

## 4. Measuring accrued profit

### 4.1 Price at NAV, never at the pool

The strategy values held RWA at the oracle NAV
(`contracts/stablebond-wrapper-strategy/src/pricing.rs:84-119`). The pool is never a pricing
input. The research measured why:

| Asset | Pool vs issuer NAV (snapshot) | Over a 90-day window |
|---|---|---|
| USDY (Aqua) | about 145 bps under | 135-150 bps under, widening |
| deJTRSY (Sushi) | about 136 bps under | about 120 bps under, despite heavy bot volume |
| USTRY (Aqua) | about 20 bps under | tracks NAV |

The discount is structural, not noise:

1. An AMM price moves only when someone trades through it, while NAV accrues every day. A
   thinly traded pool lags NAV by whatever accrued since the last trade that moved it.
2. Closing the gap requires buying in the pool and redeeming with the issuer (or minting and
   selling), which needs KYC, time and fees. Bots without issuer access only align pools with
   each other.
3. Where an issuer-side market maker is active (USTRY), the pool tracks NAV.

Pricing at the pool would therefore book a 1-1.5% loss on every purchase made at NAV and expose
NAV to anyone who can push the pool. Pricing at NAV makes a buy from a pool under NAV accretive
(it fills above the oracle-implied minimum) and keeps NAV independent of pool manipulation. For
the same reason no preset uses a pool-derived feed: the Reflector Stellar feed is computed from
Stellar DEX and pool trades, so it inherits the pool's discount and manipulability.

### 4.2 Bounding the measurement

The NAV the strategy reports is the minimum of the live oracle readings, clamped to within
`max_spread_bps` of an anchor that moves at most once per `ANCHOR_WINDOW_SECONDS` (3,600 s)
(`contracts/stablebond-wrapper-strategy/src/pricing.rs:95-101`, `:126-152`). Profit therefore
accrues at the oracle's pace, but no reading, however wrong, can change booked profit by more
than `max_spread_bps` per hour. At the presets' 10 bps that is about 2.4% per day compounded,
against a real NAV drift of roughly 0.01% per day. Freeze and reanchor controls complete the
picture (see [`architecture.md`](./architecture.md), section 6).

### 4.3 Profit, realized and unrealized

| Quantity | Definition | Where |
|---|---|---|
| Principal (per vault) | USDC deposited, converted to strategy shares at the NAV of the moment | `contracts/stablebond-wrapper-strategy/src/lib.rs:186-196` |
| Position value | `shares x (NAV + 1) / (total_shares + VIRTUAL_SHARES)`, floored | `contracts/stablebond-wrapper-strategy/src/shares.rs:53-61` |
| Accrued (unrealized) profit | Position value minus principal net of withdrawals; booked by the vault as the change in reported balance | vault report |
| Realized profit | USDC actually paid out on withdrawal or fee distribution | `contracts/stablebond-wrapper-strategy/src/lib.rs:273` |

The vault charges its fee on accrued profit, then pays the fee in USDC drawn from the
strategy's idle buffer. Accrued profit becomes realized for the user when they withdraw.

Two details keep the measurement free of artifacts:

- A deposit reports the pre-swap NAV plus the deposited amount
  (`contracts/stablebond-wrapper-strategy/src/lib.rs:202-205`) and a withdrawal the pre-trade
  NAV minus the amount paid (`:282-289`). The strategy's own trades never show up as a phantom
  gain or loss in the vault's report.
- `balance` writes nothing and cannot revert on an oracle or pool failure
  (`contracts/stablebond-wrapper-strategy/src/lib.rs:294-303`); with no live oracle it reports at
  the last anchor, so the vault keeps working and simply sees no new profit.

## 5. Revenue sharing

### 5.1 Mechanism

The split is the DeFindex vault's fee system, configured per vault:

| Parameter | Meaning | Set by |
|---|---|---|
| `vault_fee` (bps) | Share of each strategy's gains taken as a performance fee | Vault creator; vault manager can change it |
| `defindex_protocol_rate` (bps) | Share **of that fee** paid to the protocol receiver; at most 9,000 | DeFindex factory, copied into the vault at creation |
| Vault fee receiver | Receives the rest of the fee | Vault manager |
| Protocol receiver | Receives the protocol share | DeFindex factory |

Mapped onto the three participants:

```
gross gain G (reported by the strategy, booked per strategy by the vault)
  infrastructure provider  = G x vault_fee x defindex_protocol_rate
  partner application      = G x vault_fee x (1 - defindex_protocol_rate)
  depositors               = G x (1 - vault_fee)            via the vault share price
```

Fees are locked on every vault deposit and withdrawal before shares are minted or burned, so
each depositor pays the fee only on gains earned while they held. Losses stay booked and must be
recovered before any new fee accrues, which acts as a per-strategy high-water mark.

### 5.2 Worked example

Illustrative parameters: `vault_fee` = 1,000 bps (10%), `defindex_protocol_rate` = 2,000 bps
(20% of the fee). One USDC vault with one strategy. The vault's 1,000-share minimum liquidity
and integer rounding are ignored for readability.

**Day 0.** Alice deposits 1,000,000 USDC. The vault invests it in the strategy, which keeps
100,000 USDC idle as a withdrawal buffer and holds 750,000 RWA bought at NAV 1.2000 (900,000
USDC). Reported balance: 1,000,000. Alice holds 1,000,000 vault shares (price 1.0000).

**Day 90.** The RWA's NAV is 1.2120 (+1.0%). The strategy reports
`100,000 + 750,000 x 1.2120 = 1,009,000`.

| Step | Value |
|---|---|
| Gross gain booked by the vault | 1,009,000 - 1,000,000 = 9,000 |
| Fee locked (10%) | 900 |
| Assets backing shares | 1,009,000 - 900 = 1,008,100 |
| Share price | 1.0081 |

Note the gain is 0.9% on the position, not 1.0%: the idle buffer earns nothing. That buffer drag
is the price of liquid withdrawals (section 6).

**Day 90, Bob deposits 500,000 USDC.** The vault locks fees first (already done above), then
mints `500,000 / 1.0081 = 495,982.54` shares to Bob. Bob does not pay any part of the fee on the
first 9,000.

**Fee distribution.** `distribute_fees` withdraws 900 USDC from the strategy (from idle) and
pays:

| Receiver | Amount |
|---|---|
| Protocol receiver (infrastructure provider): 900 x 20% | 180 |
| Vault fee receiver (partner app): 900 - 180 | 720 |

Strategy balance after: 1,009,000 - 900 + 500,000 = 1,508,100.

**Day 180.** The strategy reports 1,514,100: a further gross gain of 6,000.

| Step | Value |
|---|---|
| Fee locked (10%) | 600 (120 to the protocol receiver, 480 to the partner) |
| Assets backing shares | 1,513,500 |
| Total shares | 1,495,982.54 |
| Share price | 1.0117097 |
| Alice's position | 1,000,000 x 1.0117097 = 1,011,709.70 |
| Bob's position | 495,982.54 x 1.0117097 = 501,790.30 |

Attribution of the second period's net gain of 5,400: Alice 3,609.70 (66.85%, her share of
assets when it accrued), Bob 1,790.30 (33.15%).

**Totals over 180 days.**

| Participant | Amount | Share of 15,000 gross |
|---|---|---|
| Depositors (Alice 11,709.70, Bob 1,790.30) | 13,500 | 90% |
| Partner application | 1,200 | 8% |
| Infrastructure provider | 300 | 2% |

Changing `vault_fee` changes the user/fee boundary; changing `defindex_protocol_rate` changes
the partner/infrastructure boundary. The strategy is the same contract in every case.

## 6. Liquidity

Liquidity, not the asset, is the main economic constraint. The RWAs are low-risk, but their
on-chain markets are thin compared with the position sizes a vault wants.

### 6.1 Measured pool depth

| Pool | USDC-side depth | Price impact at 10k USDC | At 100k USDC |
|---|---|---|---|
| Aqua constant product (USTRY, USDY, CETES) | about 1.0-1.2M | about 1% | about 9-10% |
| Sushi concentrated (deJTRSY) | about 2.0M | about 0.09% | about 0.48% |

The presets size `max_swap` to roughly 1% impact: 10,000 USDC on the Aqua presets and 100,000
USDC on deJTRSY (`contracts/stablebond-wrapper-strategy/scripts/deploy.sh:101`, `:112`, `:120`).
`max_invested` caps total exposure per deployment (`:102`, `:113`, `:121`).

### 6.2 The three liquidity tools

| Tool | Role | Cost | Reference |
|---|---|---|---|
| Idle buffer | Pays withdrawals and fee distributions instantly | Earns no yield; the operating target is about 10% of the position | `contracts/stablebond-wrapper-strategy/src/lib.rs:254` |
| Pool tranches | Incremental buys and sells, at most `max_swap` each, bounded by `slippage_bps` around NAV | Pool fee plus any price impact; buys below NAV are accretive | `contracts/stablebond-wrapper-strategy/src/lib.rs:405-460` |
| OTC fills | Size in either direction with a counterparty that mints and redeems with the issuer, at a price inside a band around NAV | The fill price's distance from NAV: zero at the shipped band of 0, at most `otc_max_deviation_bps` (cap 500) | `contracts/stablebond-wrapper-strategy/src/otc.rs:1-128` |

### 6.3 Cost of exits, and who pays it

A withdrawal is paid at NAV. If it is larger than idle, the shortfall is sold into the pool, and
any discount on that sale would be paid by the holders who stay, not by the one who leaves. The
presets therefore set `slippage_bps` to 0
(`contracts/stablebond-wrapper-strategy/scripts/deploy.sh:132`): a shortfall sale clears only at
NAV or better, and when the pool sits under NAV it reverts instead. The withdrawal waits until the
keeper refills idle with `divest_otc` at NAV. The trade-off is explicit: **exits never transfer
value between holders; large exits may wait for a refill.**

For comparison, a round trip through an Aqua pool in 10k tranches costs roughly 1.2-1.6% of the
position at measured depth, which is three to four months of yield on a treasury-bill asset. An
OTC round trip at the shipped band costs nothing beyond the counterparty's agreed price.

### 6.4 Operating model

- Deposits land in idle at par; `auto_invest` ships off
  (`contracts/stablebond-wrapper-strategy/src/lib.rs:148`) and the keeper invests idle above the
  buffer target on a schedule, preferring OTC at NAV and using pool tranches when the pool is at
  or under NAV.
- The keeper refills idle with `divest_otc` after large exits.
- Under stress (a drained pool, an impaired issuer) the honest sequence is: pause deposits, stop
  buying, hold, and exit through the counterparty at the bounded rate. There is deliberately no
  method that sells at a price an operator types.

## 7. Baskets

A DeFindex vault can hold several strategies on the same underlying asset. Because every
deployment of this contract reports in USDC at its own RWA's NAV, a single USDC vault can combine,
for example, a USTRY strategy, a USDY strategy and a deJTRSY strategy:

| Benefit | How |
|---|---|
| Issuer diversification | Each strategy is a separate deployment with its own RWA, oracle and pool; an issuer event affects only its leg |
| Liquidity diversification | Withdrawals unwind across strategies, each serving from its own idle buffer and pool |
| Per-strategy fee accounting | The vault books gains and locks fees per strategy, so a loss on one leg does not erase fees owed on another, and each leg must recover its own losses before charging again |
| One user experience | Users deposit and withdraw USDC once; allocation and rebalancing use the vault's existing manager tools |

Allocation weights and rebalancing are vault-level decisions; the strategies need no knowledge of
each other.
