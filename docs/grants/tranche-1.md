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

| Deliverable | Status | Evidence / next step |
|---|---|---|
| Research and design: compatible assets, how yield accrues, how it is measured | To write | `docs/strategies/rwa-yield-sharing/economic-design.md` |
| Architecture: attributing yield per deposit, configurable revenue share | To write | `docs/strategies/rwa-yield-sharing/architecture.md` |
| Threat model: oracle manipulation, accounting, timing attacks, mitigations | To write | `docs/strategies/rwa-yield-sharing/threat-model.md` |
| Strategy implementation | Done | `contracts/rwa-yield-sharing`, `cargo test`: 145 passed |
| Track principal and accrued yield, calculate realized profit | Done (strategy shares + oracle NAV) | Explain in architecture doc |
| Distribute configurable revenue shares | Via vault fees | Show in demo step 5 and in the dApp |
| Compatible with partner integrations | Via DeFindex vault interface | Architecture doc, dApp |
| Testnet deployment | To do | See "Testnet setup" below; addresses go in `testnet.contracts.json` |
| Test scenarios: deposits, accrual over time, attribution and sharing | To do | `demo/rwa-yield-sharing/`, tx hashes in `run-log.md` |
| Testnet dApp | To do | `app/` |
| Publish documentation | Open question | The repo is private; decide where the docs are published |

## Testnet setup (to do, in order)

1. **RWA token**: issue a test asset from a demo issuer account and deploy its SAC.
2. **Oracle**: the strategy reads `decimals()` and prices live at construction, so it needs a
   SEP-40 feed we control to simulate NAV accrual. Write a small `contracts/mock-sep40-oracle`
   with an admin-set price.
3. **Venue**: the constructor checks the pool's token set live. Check whether Aquarius runs a
   testnet router; if not, deploy the vendored Aquarius wasm from `external-contracts/` and
   create a USDC/RWA pool with liquidity.
4. **Strategy**: `contracts/stablebond-wrapper-strategy/scripts/deploy.sh testnet ... custom` with the
   addresses above.
5. **Vault**: create a DeFindex vault on testnet through the factory
   (`CDSCWE4GLNBYYTES2OCYDFQA2LLY4RBIAX6ZI32VSUXD7GO6HRPO4A32`, from DeFindex's public testnet
   contract list) with this strategy and a non-zero vault fee; add the vault as a depositor and
   its wasm hash to the strategy allowlist. Verify the factory's create-vault arguments and the
   fee receivers against the vault code before scripting it.
6. **Demo run**: two depositors, keeper invest, NAV raised in steps, fees reported and
   distributed, withdrawals. Record every tx hash.

## Open questions

- Where to publish the documentation (public repo, docs site, or a public page per tranche).
- Which testnet USDC to use (an existing testnet USDC SAC or our own test asset).
