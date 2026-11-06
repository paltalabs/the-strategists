# The Strategists

Open, composable DeFi strategy contracts on Soroban. Each strategy is an independent contract
that implements the DeFindex strategy interface, so any DeFindex vault can plug it in, and any
wallet, app or protocol on Stellar can reach it through that vault.

Revenue sharing between the end user, the partner app and the infrastructure provider happens
at the vault level: the vault's fee configuration splits realized yield, and the strategy's job
is to make that yield measurable as growth of the vault's base asset.

**Live testnet demo:** https://the-strategists-demo.vercel.app (claim test tokens, deposit,
withdraw, watch the revenue split). Deployment and scenario: [`demo/rwa-yield-sharing`](demo/rwa-yield-sharing).

## Strategies

| Strategy | Path | Status |
|---|---|---|
| RWA Yield-Sharing (tokenized stablebonds: USTRY, USDY, deJTRSY, CETES) | [`contracts/stablebond-wrapper-strategy`](contracts/stablebond-wrapper-strategy) | Deployed on testnet, live demo |
| XLM and stablecoin liquidity on Aquarius | `contracts/aquarius-lp` (planned) | Planned |
| Bridge liquidity on Allbridge | `contracts/allbridge-lp` (planned) | Planned |

## Layout

```
contracts/<strategy>/            Soroban contract, its tests and its operational scripts
external-contracts/              vendored wasm fixtures for integration tests
<network>.contracts.json         deployed addresses (written by the deploy scripts)
demo/<strategy>/                 testnet scenario and its transaction log
testnet/                         deploy, demo and keeper scripts, test token and faucet contracts
app/                             testnet dApp to deposit, withdraw and inspect vaults
docs/strategies/<strategy>/      architecture, economic design, threat model
```

## Build and test

```bash
cargo test                       # every contract
stellar contract build           # wasm32v1-none release builds into target/
```

Requires Rust with the `wasm32v1-none` target and the `stellar` CLI.

## License

[MIT](LICENSE)
