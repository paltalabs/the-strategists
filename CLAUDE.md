# the-strategists

Private repo. Soroban strategy contracts for DeFindex vaults, one per directory under
`contracts/`, plus a testnet demo dApp in `app/`.

- Read `docs/grants/tranche-1.md` first: it lists what is done and what is left.
- Each strategy has `docs/strategies/<name>/` (architecture, economic design, threat model).
  Change the doc in the same commit as the code.
- Do not mention any other repository in code, comments or docs.
- `cargo test` at the root runs every contract's suite; integration tests load wasm from
  `external-contracts/`.
