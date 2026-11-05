# RWA Yield-Sharing Strategy: threat model and risk analysis

This document lists what can go wrong with the strategy, what the contract does about each threat,
and what risk remains. It assumes the architecture in [`architecture.md`](./architecture.md) and
the economics in [`economic-design.md`](./economic-design.md).

Every mitigation cites the code as `file:line` relative to the repository root. Where a
mitigation is operational (monitoring, keeper behavior) rather than enforced by the contract, it
says so.

## 1. Trust model and security assumptions

### 1.1 Assets at risk

- USDC held idle by the strategy.
- RWA tokens held by the strategy.
- The correctness of `balance()`, which the DeFindex vault uses to price its own shares and to
  book gains that fees are charged on.

### 1.2 Actors and what is trusted

| Actor | Trust assumption | What bounds it if the assumption fails |
|---|---|---|
| DeFindex vault | Calls the trait correctly and is the only depositor | Deposit allowlist (`contracts/stablebond-wrapper-strategy/src/lib.rs:61-69`); shares keyed per vault (`contracts/stablebond-wrapper-strategy/src/storage.rs:144-145`); withdraw pays only the caller's own shares |
| Admin (multisig) | Honest majority of signers | Lower-only immediate setters, 72 h public timelock on anything that adds trust or widens a bound, no venue setter, no price setter, no arbitrary payee (section 6) |
| Keeper (bot key) | Runs flows on schedule, freezes on alerts | Every keeper action is capped and priced by the contract; no recipient choice; one 24 h freeze; replaceable by the admin in one transaction (section 6) |
| NAV oracles | Report the issuer's NAV within the source's TTL | Minimum of sources, anchor clamp (`max_spread_bps` per hour), staleness, `MAX_RATE`, freeze (section 2) |
| Venue (pool) | Executes swaps honestly | Not a pricing input; every swap bounded by `min_out` derived from the oracle; result re-checked by the strategy (section 3) |
| OTC counterparty | Fills at the agreed price | Allowlisted only after a 72 h public proposal; price must sit inside a band around the oracle rate (ships 0); both legs atomic (section 6) |
| RWA issuer | Honors NAV and redemptions; does not claw back | Cannot be bounded on-chain; detected by a balance tripwire (section 7) |
| Stellar / Soroban host | Correct execution, auth and storage | Assumed |

### 1.3 Security assumptions

1. The admin key is a multisig and its signers check proposals before executing them.
2. Someone (the keeper bot, the vault manager, or both) watches the governance and pricing events
   listed in section 9 and can act within the 72 h timelock and within a 24 h keeper freeze.
3. At least one configured oracle reports the issuer's NAV, not a market price. The presets use
   one RedStone NAV adapter per deployment.
4. The oracle's USD price is treated as a USDC price; there is no separate USDC peg check
   (`contracts/stablebond-wrapper-strategy/src/oracles.rs:80`).
5. Before the first deposit, the operator checks that the oracle matches the issuer's published
   NAV, because the first anchor is accepted unclamped
   (`contracts/stablebond-wrapper-strategy/src/pricing.rs:140`).
6. The keeper extends the TTL of the oracle and venue contracts as part of its loop
   (`contracts/stablebond-wrapper-strategy/scripts/deploy.sh:244-246`).

### 1.4 Invariants the contract enforces

- `balance()` never writes state and never reverts because of an oracle or venue failure
  (`contracts/stablebond-wrapper-strategy/src/lib.rs:294-303`).
- `withdraw` is never paused, never allowlist-gated and works in every pricing mode
  (`contracts/stablebond-wrapper-strategy/src/lib.rs:234-290`).
- The pool price never enters NAV (`contracts/stablebond-wrapper-strategy/src/pricing.rs:1-11`,
  `:84-108`).
- NAV moves at most `max_spread_bps` per `ANCHOR_WINDOW_SECONDS` through the oracle path, and at
  most one additional `MAX_REANCHOR_STEP_BPS` downward step per window through the admin's
  reanchor (`contracts/stablebond-wrapper-strategy/src/pricing.rs:126-152`, `:170-201`).
- Tokens leave only to the vault-supplied `to`, the fixed pool, or a timelock-allowlisted OTC
  counterparty (`contracts/stablebond-wrapper-strategy/src/lib.rs:273`,
  `contracts/stablebond-wrapper-strategy/src/venue.rs:54-66`,
  `contracts/stablebond-wrapper-strategy/src/otc.rs:75`, `:116`).

## 2. Oracle manipulation and inaccurate pricing feeds

### T1. A wrong or manipulated oracle price

- **Description.** An oracle source reports a price far from the true NAV, through a bug, an
  operator error, a compromised or upgraded adapter, or manipulation of whatever the feed reads.
- **Impact.** Unbounded, the strategy would book a false gain (fees charged on yield that does not
  exist, new vault depositors overpaying) or a false loss (exiting holders underpaid, new
  depositors buying cheap), and the keeper's buys would be sized at a wrong price.
- **Mitigation in the contract.**
  - The rate is the **minimum** of all live sources
    (`contracts/stablebond-wrapper-strategy/src/pricing.rs:64-74`).
  - The rate is clamped to `[L*(1-s), L*(1+s)]` around the anchor `L`, and `L` is rewritten at
    most once per `ANCHOR_WINDOW_SECONDS` (3,600 s), only in `Live`
    (`contracts/stablebond-wrapper-strategy/src/pricing.rs:96-99`, `:136-140`;
    `contracts/stablebond-wrapper-strategy/src/constants.rs:25`). A wrong feed moves NAV at most
    `max_spread_bps` per hour: about 2.4% per day at the presets' 10 bps.
  - `max_spread_bps` is bounded to [10, 500] and can be raised only after a 72 h public proposal
    (`contracts/stablebond-wrapper-strategy/src/constants.rs:14-15`,
    `contracts/stablebond-wrapper-strategy/src/lib.rs:688-690`).
  - Buys refuse while the oracle reads below the clamped rate, so the strategy never pays more
    than the oracle's own price (`contracts/stablebond-wrapper-strategy/src/pricing.rs:79-81`).
  - `PriceJumpEvent` fires on every guarded call while the raw rate is more than one spread from
    the anchor (`contracts/stablebond-wrapper-strategy/src/pricing.rs:133-135`).
  - `freeze_price` holds the rate at the anchor immediately, by the keeper (24 h, once) or the
    admin (until lifted) (`contracts/stablebond-wrapper-strategy/src/lib.rs:709-728`). While
    frozen, deposits against held RWA and all buys refuse.
  - The bad source can be replaced through `propose_oracles` / `execute_oracles`; execution needs
    only one live proposed source, so a correct feed can always replace a wrong one however far
    the anchor walked (`contracts/stablebond-wrapper-strategy/src/oracles.rs:92-99`).
- **Residual risk.** With one oracle per deployment, nothing on-chain disagrees with it. A feed
  that walks at or under one spread per hour never triggers `PriceJumpEvent`; only an off-chain
  comparison of the oracle against the issuer's published NAV catches it. That monitoring is a
  security control and must be live before the first deposit. Until someone freezes, NAV can drift
  by up to the hourly bound.

### T2. Stale, dead or reverting oracle

- **Description.** The feed stops updating, the adapter is archived, paused or upgraded to
  something that reverts, or returns zero or an absurd value.
- **Impact.** A reverting `balance()` would block every deposit and withdrawal of the vault (which
  reads every strategy's balance). A stale price would freeze accrual or misprice entries.
- **Mitigation in the contract.**
  - Every oracle call is a `try_` call; failure, a non-positive price, a timestamp older than the
    source's `ttl_seconds`, or a normalized rate of 0 or above `MAX_RATE` all count as "not live"
    (`contracts/stablebond-wrapper-strategy/src/oracles.rs:61-83`,
    `contracts/stablebond-wrapper-strategy/src/constants.rs:58-64`).
  - TTLs are bounded to `(0, MAX_TTL_SECONDS]` = 24 h at construction and on proposals
    (`contracts/stablebond-wrapper-strategy/src/oracles.rs:33-50`).
  - With no live source the mode is `OracleDown` and the rate is the anchor: `balance()` still
    returns, withdrawals and sells continue at the anchor, deposits against held RWA and all buys
    refuse (`contracts/stablebond-wrapper-strategy/src/pricing.rs:102-105`,
    `contracts/stablebond-wrapper-strategy/src/lib.rs:180-182`).
  - `PricingModeEvent` and `SpreadAlertEvent` signal the degraded mode
    (`contracts/stablebond-wrapper-strategy/src/pricing.rs:142-150`,
    `contracts/stablebond-wrapper-strategy/src/lib.rs:224-226`).
- **Residual risk.** NAV stops accruing while the oracle is down. With no anchor and no live
  oracle, held RWA is valued at zero (the most conservative number, still non-reverting). A feed
  whose heartbeat equals 24 h cannot be given any margin under `MAX_TTL_SECONDS`, so each
  adapter's live heartbeat must be checked before deployment.

### T3. A real, fast NAV move

- **Description.** NAV genuinely moves faster than the clamp allows: an issuer impairment, a
  currency move on a non-USD bond such as CETES, or a depeg.
- **Impact.** Booked NAV lags the truth by up to the size of the move. On the way down, exiting
  holders are paid above true value at the expense of those who stay; on the way up, entrants buy
  below true value.
- **Mitigation in the contract.**
  - Buys refuse while the oracle reads below the clamped rate
    (`contracts/stablebond-wrapper-strategy/src/pricing.rs:79-81`).
  - `reanchor_to_oracle` lets the admin book a confirmed drop in steps of at most
    `MAX_REANCHOR_STEP_BPS` (500 bps) per window, only downward, only to a rate every configured
    oracle confirms and that matches the multisig's checked figure
    (`contracts/stablebond-wrapper-strategy/src/pricing.rs:170-201`,
    `contracts/stablebond-wrapper-strategy/src/constants.rs:72`).
  - Withdrawals above idle need a pool sale bounded by `slippage_bps` around the (lagging) rate,
    which limits how much can leave at the stale price quickly
    (`contracts/stablebond-wrapper-strategy/src/flows.rs:25-28`).
- **Residual risk.** Accepted. The clamp trades speed for manipulation resistance. For the target
  assets, real NAV moves are far smaller than the clamp; an issuer impairment is an asset risk
  (T18) that no pricing rule recovers.

## 3. Pool price manipulation

### T4. Pushing the pool to move NAV or extract value from swaps

- **Description.** An attacker with capital moves the fixed pool's price, possibly around a keeper
  tranche or a withdrawal shortfall sale (sandwich).
- **Impact.** If the pool priced NAV, the attacker could inflate or deflate vault share prices. If
  swaps were bounded by the pool's own quote, a sandwich could extract the full slippage
  tolerance.
- **Mitigation in the contract.**
  - The pool is never a pricing input (`contracts/stablebond-wrapper-strategy/src/pricing.rs:1-11`,
    `:84-108`). A pushed pool moves neither NAV nor the buy gate.
  - Buys require at least the oracle-implied RWA minus `slippage_bps`
    (`contracts/stablebond-wrapper-strategy/src/flows.rs:14-17`); a pool pushed above NAV makes
    the buy revert, and a pool pushed below NAV makes it cheaper.
  - Sales give up at most the oracle-implied RWA plus `slippage_bps` and require the full USDC
    target back (`contracts/stablebond-wrapper-strategy/src/flows.rs:25-28`,
    `contracts/stablebond-wrapper-strategy/src/lib.rs:268`, `:456`).
  - The strategy re-checks `received >= min_out` itself after each swap
    (`contracts/stablebond-wrapper-strategy/src/venue_aqua.rs:48-50`,
    `contracts/stablebond-wrapper-strategy/src/venue_sushi.rs:73-75`).
  - Each swap is at most `max_swap`, and `max_swap` can only be raised after a 72 h public
    proposal (`contracts/stablebond-wrapper-strategy/src/lib.rs:666-668`).
  - The auto-invest tranche pre-checks the pool's quote against the oracle-implied minimum and
    skips silently if it falls short (`contracts/stablebond-wrapper-strategy/src/flows.rs:64-68`).
- **Residual risk.** A sandwich can capture at most `slippage_bps` of a trade relative to NAV. At
  the presets' 0 the strategy only trades at NAV or better, so the remaining exposure is the pool
  fee. The attacker can make trades revert (denial of service on pool tranches), which the OTC
  path and the idle buffer absorb.

### T5. Malicious or broken venue

- **Description.** The pool's admin pauses or upgrades it, or it returns a wrong swap result.
- **Impact.** Swaps fail or return less than expected.
- **Mitigation in the contract.** The venue only executes swaps. The strategy authorizes exactly
  one `transfer` of the input amount to the pool for each swap
  (`contracts/stablebond-wrapper-strategy/src/venue.rs:54-66`) and re-checks the output. A failed
  swap is a typed `InternalSwapError`. Withdrawals continue from idle, and `divest_otc` exits
  without the pool. The venue is immutable, so it cannot be swapped for a malicious one
  (`contracts/stablebond-wrapper-strategy/src/storage.rs:55-56`).
- **Residual risk.** A dead pool removes the pool exit path until a counterparty refills idle. A
  pool migration requires a new strategy deployment and a vault rebalance.

## 4. Accounting inconsistencies in yield attribution

### T6. First-depositor inflation (share price manipulation by donation)

- **Description.** The first depositor mints a tiny number of shares, then donates assets to the
  strategy to inflate the share price, so the next depositor's mint rounds down to nothing.
- **Impact.** Theft of the next deposit.
- **Mitigation in the contract.** Virtual offset of `VIRTUAL_SHARES` = 1,000,000 on the share side
  and +1 on the asset side of every conversion
  (`contracts/stablebond-wrapper-strategy/src/shares.rs:15-27`,
  `contracts/stablebond-wrapper-strategy/src/constants.rs:6`); a mint of zero shares reverts
  (`contracts/stablebond-wrapper-strategy/src/lib.rs:188-190`); only allowlisted vaults can
  deposit at all. The DeFindex vault adds its own minimum-liquidity guard at its level.
- **Residual risk.** None material: the attack costs the donor far more than it can take.

### T7. Rounding drift between holders

- **Description.** Repeated conversions round in a direction that leaks value from remaining
  holders to exiting ones.
- **Impact.** Slow dilution of long-term holders.
- **Mitigation in the contract.** Floor on mint, ceil on burn, floor on reported value
  (`contracts/stablebond-wrapper-strategy/src/shares.rs:29-61`); full-exit burn clamped to the
  vault's holding (`contracts/stablebond-wrapper-strategy/src/lib.rs:250-252`). Trade sizing that
  gives tokens away rounds up against the counterparty
  (`contracts/stablebond-wrapper-strategy/src/math.rs:75-87`,
  `contracts/stablebond-wrapper-strategy/src/math.rs:128-136`); the OTC band rounds both edges
  inward (`contracts/stablebond-wrapper-strategy/src/otc.rs:26-32`).
- **Residual risk.** One exception runs the other way: an `invest_otc` fill receives the floored
  RWA quantity for the USDC paid, favoring the counterparty by under one raw RWA unit per fill.
  Negligible.

### T8. Phantom gains or losses in the vault's report

- **Description.** The vault books a strategy's gain as the change in reported balance net of its
  own flows. If the strategy's reported balance after a deposit or withdrawal included its own
  swap's price impact, the vault would book a false gain (and charge fees on it) or a false loss.
- **Impact.** Fees charged on non-existent yield, or yield hidden from the fee split.
- **Mitigation in the contract.** `deposit` returns the balance from the pre-swap NAV plus the
  deposited amount (`contracts/stablebond-wrapper-strategy/src/lib.rs:202-205`); `withdraw`
  returns the pre-trade NAV minus the amount paid (`:282-289`). Deposits enter idle at par, so the
  deposited USDC adds exactly its amount to NAV. Pool trades are bounded around NAV, so a keeper
  trade cannot move booked NAV beyond `slippage_bps` of the traded size.
- **Residual risk.** A keeper pool buy below NAV is booked as a gain on the next report (it is a
  real gain, and is fee'd as such). At `slippage_bps` 0, no pool trade books a loss.

### T9. Arithmetic overflow and precision

- **Description.** Large positions in 18-decimal RWAs, or an absurd oracle value, overflow `i128`
  or truncate.
- **Impact.** A reverting `balance()` (blocking the vault) or mispriced shares.
- **Mitigation in the contract.** All price-scale intermediates use `I256`
  (`contracts/stablebond-wrapper-strategy/src/math.rs:1-2`, `:35-87`); every other operation is
  checked. Rates above `MAX_RATE = i128::MAX / 20_000` count as not live, which keeps the bps
  helpers in range for every rate pricing can see
  (`contracts/stablebond-wrapper-strategy/src/constants.rs:58-64`). Decimals are capped at 18 so
  `pow10` stays bounded (`contracts/stablebond-wrapper-strategy/src/constants.rs:51-53`,
  `contracts/stablebond-wrapper-strategy/src/math.rs:10-22`).
- **Residual risk.** None known.

### T10. Fee distribution draws on the buffer

- **Description.** The vault pays locked fees by withdrawing USDC from the strategy. That
  withdrawal is served from idle first, and a shortfall goes to the pool like any other withdrawal.
- **Impact.** If idle is short and the pool is under NAV, fee distribution reverts.
- **Mitigation in the contract.** The same withdrawal rules apply: no value leaves below NAV
  (`contracts/stablebond-wrapper-strategy/src/lib.rs:254-270`).
- **Residual risk.** A liveness delay for fee payouts, not a loss; the fee stays locked and is paid
  after the keeper refills idle.

## 5. Timing attacks around deposits and withdrawals

### T11. Entering just before NAV rises, leaving just after

- **Description.** A vault user deposits right before a known NAV update (or while the clamp is
  catching up to a higher oracle reading) and withdraws right after, capturing the step.
- **Impact.** The step's value is shared with someone who did not hold during the accrual,
  diluting existing holders.
- **Mitigation in the contract.**
  - New strategy shares mint at the **pre-deposit** NAV and the USDC enters at par
    (`contracts/stablebond-wrapper-strategy/src/lib.rs:186-187`), so the entrant only shares in
    increases booked after entry.
  - Booked NAV moves at most `max_spread_bps` per hour
    (`contracts/stablebond-wrapper-strategy/src/pricing.rs:96-99`, `:136-140`), so no single step
    exceeds that bound.
  - Deposits against held RWA refuse unless pricing is `Live`
    (`contracts/stablebond-wrapper-strategy/src/lib.rs:180-182`), so no one can enter at a stale
    anchor while the oracle is down or frozen and exit after it recovers.
  - At the vault level, fees are locked before shares are minted or burned, so an entrant does not
    escape fees on gains booked during their stay.
- **Residual risk.** Small. NAV feeds update in steps of a few bps, so the capturable step is a
  fraction of a basis point to a few bps of the entrant's capital, against transaction costs and
  the vault fee. If the clamp is binding upward (the oracle above the anchor), an entrant can
  capture up to the remaining gap, bounded by how far the anchor lags; the keeper sees this as
  `raw_rate != rate` in `get_pricing_state`. Vault deposits that stay idle in the vault are still
  priced from `balance()` at the anchor while the strategy is `OracleDown` or `Frozen`; that is a
  vault-level exposure bounded by the same anchor lag.

### T12. Leaving just before NAV falls

- **Description.** A holder who sees the oracle drop (or an impairment coming) withdraws while
  booked NAV still lags above the truth.
- **Impact.** The exiting holder is paid above true value; the loss is concentrated on holders who
  stay.
- **Mitigation in the contract.**
  - `reanchor_to_oracle` books a confirmed drop in steps of up to 500 bps per window
    (`contracts/stablebond-wrapper-strategy/src/lib.rs:741-749`).
  - Exits beyond idle need a pool sale of at most `max_swap`, bounded around the rate
    (`contracts/stablebond-wrapper-strategy/src/lib.rs:258-268`), which limits how much can leave
    at the lagging price per transaction.
  - A freeze holds the anchor; it does not help here, which is why the admin, not the keeper, is
    expected to decide between freezing and reanchoring.
- **Residual risk.** Accepted. Up to one clamp step plus the time it takes the multisig to reanchor
  can be captured by fast exits from the idle buffer. Withdrawals are deliberately never paused,
  so this cannot be closed without trapping honest holders.

### T13. Sandwiching keeper tranches and auto-invest

See T4. A keeper or auto-invest buy has an oracle-derived floor and a sale an oracle-derived cap,
so ordering transactions around them can make them revert but cannot make them trade worse than
`slippage_bps` from NAV.

## 6. Privileged roles

### T14. Compromised keeper key

- **Description.** An attacker controls the keeper.
- **Impact.** The keeper trades, freezes and pauses.
- **Mitigation in the contract.**
  - Every keeper method requires keeper auth before any state change
    (`contracts/stablebond-wrapper-strategy/src/lib.rs:214-217`, `:408`, `:438`, `:469`, `:483`).
  - No keeper method names a recipient: pool proceeds stay idle, OTC pays only an allowlisted
    counterparty (`contracts/stablebond-wrapper-strategy/src/otc.rs:54-56`, `:101-103`).
  - Pool trades are capped by `max_swap`, `max_invested` and idle, and priced by
    `slippage_bps` around the oracle rate (`contracts/stablebond-wrapper-strategy/src/lib.rs:405-460`).
  - OTC prices must sit inside `otc_max_deviation_bps` of the rate, which ships at 0, so a fill
    happens at exactly the rate (`contracts/stablebond-wrapper-strategy/src/otc.rs:35-41`).
  - The keeper's freeze lasts `KEEPER_FREEZE_SECONDS` (24 h) and cannot be renewed until the
    admin re-arms it; the keeper cannot unfreeze
    (`contracts/stablebond-wrapper-strategy/src/lib.rs:718-724`).
  - The admin replaces the keeper immediately (`contracts/stablebond-wrapper-strategy/src/lib.rs:643-653`).
  - Pausing affects only deposits and buys, never withdrawals or sells
    (`contracts/stablebond-wrapper-strategy/src/lib.rs:314-327`).
- **Residual risk.** Churn costs (pool fees) until rotation. With a colluding counterparty, OTC
  fills anywhere in the current band: nothing at the shipped 0, at most the band per fill
  otherwise. A keeper outage stops new investment, never withdrawals.

### T15. Compromised admin multisig

- **Description.** An attacker controls the admin.
- **Impact.** Could try to change prices, venues, counterparties or recipients to extract funds.
- **Mitigation in the contract.**
  - No venue setter and no price setter; no method pays an address the admin chooses without a
    72 h public proposal (`contracts/stablebond-wrapper-strategy/src/storage.rs:55-56`,
    `contracts/stablebond-wrapper-strategy/src/timelock.rs:142-161`).
  - `max_swap`, `max_spread_bps`, `slippage_bps` and the OTC band can only be **lowered**
    immediately; raising any of them needs `propose_params`, announced by `ParamsProposedEvent`
    with every value, and `execute_params` after `TIMELOCK_SECONDS` (72 h)
    (`contracts/stablebond-wrapper-strategy/src/lib.rs:563-565`, `:666-668`, `:688-690`;
    `contracts/stablebond-wrapper-strategy/src/timelock.rs:70-121`).
  - Oracle sets and OTC counterparties change only through the same 72 h proposal path
    (`contracts/stablebond-wrapper-strategy/src/timelock.rs:30-55`, `:143-161`).
  - Hard caps bound every knob even after a timelock: `MAX_SPREAD_BPS`, `MAX_SLIPPAGE_BPS` and
    `MAX_OTC_DEVIATION_BPS` are all 500 (`contracts/stablebond-wrapper-strategy/src/constants.rs:15`,
    `:42`, `:49`).
  - `reanchor_to_oracle` only lowers NAV, by at most 500 bps per window, and only toward a rate
    every oracle confirms (`contracts/stablebond-wrapper-strategy/src/pricing.rs:170-201`).
  - Admin transfer is two-step, the new admin must sign
    (`contracts/stablebond-wrapper-strategy/src/lib.rs:616-638`).
- **Residual risk.**
  - At the knobs currently stored, the admin can install itself as keeper and trade against the
    pool while the rate sits up to one spread off the oracle (a freeze or a binding clamp):
    bounded per leg by roughly `(1 + slippage_bps) / (1 - max_spread_bps) - 1`, about 0.10% at the
    presets, per `max_swap`. Anything wider needs a public proposal first.
  - `set_caps(1, 1)` can strand the RWA behind a tiny swap cap; undoing it waits 72 h. Idle still
    withdraws in full and `divest_otc` is not `max_swap`-bounded, so this is a liveness cost.
  - The admin can allowlist itself as a depositor immediately; a deposit against held RWA requires
    `Live`, so any mispricing it can exploit is bounded by the spread.
  - Pending proposals never expire and survive an admin rotation, so monitors must poll
    `get_pending_oracles`, `get_pending_params` and `get_pending_otc_counterparty` continuously,
    and a new admin after a compromise must cancel what the old one proposed.
  - The 72 h buys notice, not consent: it protects depositors only if someone watches and the vault
    manager pauses or rebalances out of the strategy in time.

### T16. Malicious OTC counterparty

- **Description.** A counterparty tries to take USDC without delivering RWA, or the reverse.
- **Mitigation in the contract.** Both legs happen in one invocation: the strategy pulls the
  counterparty's leg with `transfer_from` first, then pays its own
  (`contracts/stablebond-wrapper-strategy/src/otc.rs:74-75`, `:115-116`). A missing allowance
  reverts the whole call. `divest_otc` refuses rather than clamps when the RWA leg exceeds holdings
  (`:111-113`). Removal of a counterparty is immediate
  (`contracts/stablebond-wrapper-strategy/src/timelock.rs:165-169`).
- **Residual risk.** Price risk only, bounded by the band (0 at ship).

### T17. Unauthorized deposits or withdrawals

- **Description.** A non-vault address deposits, or one vault withdraws another's shares.
- **Mitigation in the contract.** `deposit` requires the caller's auth and an allowlist entry by
  address or wasm hash (`contracts/stablebond-wrapper-strategy/src/lib.rs:169-172`, `:61-69`);
  shares are keyed by vault and `withdraw` burns only the caller's own
  (`contracts/stablebond-wrapper-strategy/src/lib.rs:237`, `:243-252`).
- **Residual risk.** A wasm-hash allowlist entry admits every contract running that code; the admin
  should allowlist vault wasm hashes only for vault code it has reviewed.

## 7. External dependencies

### T18. Issuer action: clawback, freeze, or impairment

- **Description.** The issuer claws back or freezes the strategy's RWA (USDY carries clawback and
  revocation flags; deJTRSY has an issuer admin surface), or the underlying fund is impaired.
- **Impact.** Loss of principal proportional to the affected holdings.
- **Mitigation in the contract.** An `ExpectedRwa` tripwire compares the live RWA balance with the
  balance accounted flows imply on every state-changing call and emits
  `RwaBalanceAnomalyEvent` on a shortfall, then resyncs (alert only, never revert)
  (`contracts/stablebond-wrapper-strategy/src/lib.rs:76-87`). NAV uses the live balance, so a
  clawback is reflected immediately and losses are booked to all holders pro rata.
- **Residual risk.** Not fixable on-chain. Diversifying issuers across strategies in one vault
  (basket) limits the impact of a single issuer.

### T19. Oracle provider dependency

- **Description.** The RedStone adapters are owner-upgradable; the provider could change behavior.
- **Mitigation in the contract.** Same as T1 and T2: any behavior change is bounded by the clamp,
  staleness and freeze, and the set can be replaced after 72 h.
- **Residual risk.** Same as T1.

### T20. Venue upgradeability

- **Description.** The Sushi pool is admin-upgradeable and pausable; Aqua pools are governed by
  Aquarius.
- **Mitigation in the contract.** See T5: the pool is execution only, never pricing, and every
  swap output is re-checked.
- **Residual risk.** Loss of the pool exit path until a new deployment.

### T21. Storage archival

- **Description.** On Soroban, contract instances and storage entries expire if their TTL is not
  extended. An archived oracle or pool instance cannot be called; an archived strategy entry would
  lose state.
- **Mitigation in the contract.** Instance TTL is bumped on every entry point that touches
  storage, persistent entries on read and write
  (`contracts/stablebond-wrapper-strategy/src/storage.rs:158-176`). An archived oracle reads as
  not live and degrades to `OracleDown` without reverting `balance()`.
- **Residual risk.** Operational: the keeper must extend the TTL of the oracle and venue contracts
  (`contracts/stablebond-wrapper-strategy/scripts/deploy.sh:244-246`). `balance()` is read-only
  and does not bump TTL by design.

## 8. Liquidity risk

### T22. Withdrawal larger than the buffer while the pool is under NAV

- **Description.** A vault withdrawal exceeds the strategy's idle USDC and the pool trades under
  NAV (or the shortfall exceeds `max_swap`).
- **Impact.** The withdrawal reverts with `InsufficientBalance`; since the vault collapses a
  strategy error into a failed withdrawal, the user's whole vault withdrawal waits.
- **Mitigation in the contract.** Deliberate: with `slippage_bps` 0 a shortfall sells only at NAV
  or better (`contracts/stablebond-wrapper-strategy/src/flows.rs:25-28`,
  `contracts/stablebond-wrapper-strategy/src/lib.rs:258-268`), so no exit pushes a discount onto
  the holders who stay. `divest_otc` refills idle at the rate in every mode with a rate,
  `Frozen` included (`contracts/stablebond-wrapper-strategy/src/otc.rs:93-128`).
- **Residual risk.** Liveness. Operationally mitigated by an idle buffer near 10% of the position,
  refilling after large exits, and splitting large withdrawals.

### T23. Positions larger than on-chain liquidity

- **Description.** Target positions are several times the USDC depth of the pools.
- **Impact.** A full unwind through the pool is impossible without large losses.
- **Mitigation in the contract.** `max_invested` caps exposure per deployment and `max_swap` caps
  each pool trade (`contracts/stablebond-wrapper-strategy/src/flows.rs:31-34`,
  `contracts/stablebond-wrapper-strategy/src/lib.rs:413-415`, `:440-442`). There is no method to
  sell at an arbitrary floor. Size moves through the OTC path at NAV.
- **Residual risk.** The main residual risk of the strategy, and not one a contract removes: exits
  at scale depend on the counterparty's mint and redeem capacity with the issuer, or on pool depth
  growing.

## 9. Monitoring required by the design

These alerts are part of the security model, not optional telemetry:

| Signal | Source | Action |
|---|---|---|
| `PriceJumpEvent`, `raw_rate != rate` | `contracts/stablebond-wrapper-strategy/src/events.rs:199`, `contracts/stablebond-wrapper-strategy/src/lib.rs:492` | Compare with issuer NAV; freeze if wrong |
| Oracle vs issuer-published NAV | off-chain | Freeze when they diverge by more than a few bps |
| `PricingModeEvent`, `SpreadAlertEvent` | `contracts/stablebond-wrapper-strategy/src/events.rs:187`, `:168` | Investigate oracle liveness |
| `PriceFrozenEvent` | `contracts/stablebond-wrapper-strategy/src/events.rs:254` | Admin confirms a keeper freeze within 24 h or lifts it |
| `OraclesProposedEvent`, `ParamsProposedEvent`, `OtcCounterpartyProposedEvent`, `AdminTransferStartedEvent` | `contracts/stablebond-wrapper-strategy/src/events.rs:111`, `:138`, `:158`, `:19` | Review within 72 h; vault manager pauses or exits if unexpected |
| `RwaBalanceAnomalyEvent` | `contracts/stablebond-wrapper-strategy/src/events.rs:177` | Contact issuer; assess loss |
| Idle below buffer target | `get_pricing_state`, token balances | Refill with `divest_otc` |

`OraclesProposedEvent` carries only the number of sources; monitors read the proposed set with
`get_pending_oracles` (`contracts/stablebond-wrapper-strategy/src/lib.rs:597-600`).

## 10. Summary

| Risk area | Main control | Residual |
|---|---|---|
| Oracle manipulation / wrong feed | Min of sources, anchor clamp (`max_spread_bps` per hour), freeze, timelocked replacement | Slow drift within the hourly bound until someone freezes; monitoring required |
| Pool manipulation | Pool never prices NAV; swaps bounded around the oracle | At most `slippage_bps` per trade (0 at presets) |
| Accounting / attribution | Virtual-offset shares, conservative rounding, pre-trade NAV returns, vault per-strategy reports | Sub-unit OTC rounding |
| Timing attacks | Mint at pre-deposit NAV, `Live`-only deposits against held RWA, bounded NAV steps, fees locked before mint and burn | Capture of small NAV steps; fast exits ahead of a real drop up to one clamp step |
| Privileged roles | Lower-only setters, 72 h timelock, no venue or price setter, no arbitrary payee, one-shot keeper freeze | Bounded pool extraction at current knobs by a compromised admin; requires watching |
| Issuer | Balance tripwire, basket diversification | Not fixable on-chain |
| Liquidity | Buffer, caps, OTC at NAV, no below-NAV exits | Large exits wait for refills; scale depends on counterparty capacity |
