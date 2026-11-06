# Tranche 1: Yield-Sharing Strategy for Yield-Bearing Assets

Internal tracking doc. Maps every Tranche 1 deliverable to its evidence and lists what is left.

## Completion criteria (from the submission)

1. Strategy smart contract deployed on Testnet.
2. Demonstration of deposits and automated yield distribution between participants.
3. Published documentation: strategy architecture, economic design, threat model and risk analysis.

## Decisions

- **Revenue sharing happens at the DeFindex vault level.** The strategy makes yield measurable
  as base-asset growth; the vault's fee configuration splits it between the user, the partner
  app and the infrastructure provider. No separate split contract.
- **Deposits are USDC, exposure is the RWA.** A value-accruing token never grows its balance,
  so balance-based fee accounting would see zero yield. The economic design doc explains this
  as a research finding of the tranche.
- **Demo through a DeFindex vault on testnet plus a simple testnet dApp** that shows the vault
  and strategy state and lets anyone deposit and withdraw.

## Status per deliverable

| Deliverable | Status | Evidence |
|---|---|---|
| Research and design: compatible assets, how yield accrues, how it is measured | Done | `docs/strategies/rwa-yield-sharing/economic-design.md` |
| Architecture: attributing yield per deposit, configurable revenue share | Done | `docs/strategies/rwa-yield-sharing/architecture.md` |
| Threat model: oracle manipulation, accounting, timing attacks, mitigations | Done | `docs/strategies/rwa-yield-sharing/threat-model.md` |
| Strategy implementation | Done | `contracts/stablebond-wrapper-strategy`, `cargo test`: 145 passed |
| Track principal and accrued yield, calculate realized profit | Done | Strategy shares + oracle NAV (architecture doc); vault `report` in the demo |
| Distribute configurable revenue shares | Done | Vault fee split, demo steps 4 and 7 |
| Compatible with partner integrations | Done | Standard DeFindex vault interface; testnet dApp |
| Testnet deployment | Done | `testnet.contracts.json`, `demo/rwa-yield-sharing/README.md` |
| Test scenarios: deposits, accrual over time, attribution and sharing | Done | `demo/rwa-yield-sharing/run-log.md` (every tx hash) |
| Testnet dApp | Done | `app/`: User tab (deposit, withdraw, revenue split) and Admin tab (rebalance, fees, keeper, oracle, mint) |
| Live for reviewers | Done | Faucet (`testnet/faucet`), dApp "Get test tokens", `testnet/keeper.sh loop` keeps NAV, pool, allocation and revenue share moving |
| Publish documentation | Open | The repo is private; decide where the docs are published |

## Testnet stack

`testnet/setup.sh` deploys everything idempotently; `testnet/demo.sh` runs the scenario. Testnet-only
helpers live outside the Cargo workspace so they never touch the strategy build:
`testnet/faucet` (hands out pre-minted TUSDC and TBOND, once an hour per address),
`testnet/test-token` (SEP-41 tokens: contract tokens need no trustlines, so the protocol fee
receiver, an account we do not control, can be paid) and `testnet/sushi-factory-stub` (the pool
reads its protocol fee from a factory).

Things learned on the way:

- The vault constructor traps when "DeFindex-Vault-" plus the vault name exceeds 32 characters.
- Reflector stops answering one resolution (1 h) after the last push: run `push_nav.sh` hourly
  or the strategy drops to `OracleDown`.
- `lock_fees` works on the stored report; call `report` first so it sees the latest gains.
- A withdrawal that sells RWA reverts while the pool trades more than `slippage_bps` under the
  oracle rate; the demo arbitrages the pool first (`demo.sh arb`).

## Open questions

- Where to publish the documentation (public repo, docs site, or a public page per tranche).
