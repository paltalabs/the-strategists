# RWA Yield-Sharing: testnet demo

The strategy runs behind a DeFindex vault on Stellar testnet. Two users deposit, the RWA's NAV
rises over two periods, the vault splits the realized yield between the users, the partner app
and the infrastructure provider, and both users withdraw. Every transaction is in
[`run-log.md`](run-log.md) with a stellar.expert link.

## Deployment (testnet)

| Contract | Address |
|---|---|
| DeFindex vault (`STRWA`) | [`CA5ZCTDCHQVFBMXJTQ62AMAA7QX37F672TMZXGRFKAJZQ57N5X5IQX3T`](https://stellar.expert/explorer/testnet/contract/CA5ZCTDCHQVFBMXJTQ62AMAA7QX37F672TMZXGRFKAJZQ57N5X5IQX3T) |
| RWA yield-sharing strategy | [`CCHL22QTS4BCVKBBRBITOSOONJRANKOO6MZJHBTWU4J7BMX4RI2QX5Y6`](https://stellar.expert/explorer/testnet/contract/CCHL22QTS4BCVKBBRBITOSOONJRANKOO6MZJHBTWU4J7BMX4RI2QX5Y6) |
| TUSDC (test USDC, SEP-41) | [`CDILICOGC4VMXADDCNQK2V2GAPAHLPULYVOOQ6KY36ZVJ5FDXNCLU3KW`](https://stellar.expert/explorer/testnet/contract/CDILICOGC4VMXADDCNQK2V2GAPAHLPULYVOOQ6KY36ZVJ5FDXNCLU3KW) |
| TBOND (test tokenized bond, SEP-41) | [`CCIHNSP55BIAHKKDWU2JBXCKYR6WNZI52CVYUR6EL5V6SWGKNRS6DTVK`](https://stellar.expert/explorer/testnet/contract/CCIHNSP55BIAHKKDWU2JBXCKYR6WNZI52CVYUR6EL5V6SWGKNRS6DTVK) |
| Reflector SEP-40 oracle (TBOND NAV in TUSDC) | [`CCOZLM6SPEDFUQ4CDRDKGFK35SE2DU6ZIGFYXLR5D5JNKRNU6MPFV6PL`](https://stellar.expert/explorer/testnet/contract/CCOZLM6SPEDFUQ4CDRDKGFK35SE2DU6ZIGFYXLR5D5JNKRNU6MPFV6PL) |
| Sushi concentrated pool TBOND/TUSDC | [`CB2BMNBTHCEKFFUTR53UP6OI674D7SZ7NPIGPA7KPC4U43VWJOJQX6Y3`](https://stellar.expert/explorer/testnet/contract/CB2BMNBTHCEKFFUTR53UP6OI674D7SZ7NPIGPA7KPC4U43VWJOJQX6Y3) |
| Test token faucet | [`CAH24OTCYNWGWO7EMXQOIBDKFCOM4UQDMGMHS45YQFZWQXE4CM7C455V`](https://stellar.expert/explorer/testnet/contract/CAH24OTCYNWGWO7EMXQOIBDKFCOM4UQDMGMHS45YQFZWQXE4CM7C455V) |

Strategy wasm sha256: `3900fb6f1c2ec703eeacce3f9edd86cc23ebc5bcfed2eda15d9be7889e17fc1b`, built
from `contracts/stablebond-wrapper-strategy` with `stellar contract build`.

Strategy parameters: `max_swap` 10,000 TUSDC, `max_invested` 1,000,000 TUSDC, `max_spread_bps` 500
(the contract maximum, so a NAV step books within one anchor window), `slippage_bps` 100, one
Reflector source with a 24 h TTL.

Vault: `vault_fee` 2,000 bps (20% of realized gains). The factory's protocol rate is 2,000 bps of
that fee. Fee receiver (partner app) `GCEXO5KD…B4PV`; protocol receiver (infrastructure provider)
`GDOFDSMF…FLPJ`.

## Scenario and results

| Step | What happened |
|---|---|
| 1. Deposits | Alice deposits 5,000 TUSDC, Bob 3,000 TUSDC. |
| 2. Allocation | The vault manager moves 6,000 TUSDC into the strategy; the keeper buys 4,974.9 TBOND for 5,000 TUSDC on the pool, keeping 1,000 TUSDC idle. |
| 3. Yield, period 1 | TBOND NAV goes from 1.00 to 1.03 on the oracle. The strategy's balance for the vault goes from 6,000.00 to 6,124.15 TUSDC. |
| 4. Revenue share, period 1 | The vault books a 124.15 gain and locks a 24.83 fee. Distribution pays the partner 19.86 and the protocol 4.97. Depositors keep 99.32 through the share price. |
| 5. Alice exits | Alice redeems all her shares for 5,062.07 TUSDC: +62.07, her 5/8 of the depositors' 99.32. |
| 6. Yield, period 2 | NAV rises another 2% (1.0506). The strategy prices it at 1.05: the anchor clamp limits each window's move to `max_spread_bps`. |
| 7. Revenue share, period 2 | The vault books a 59.62 gain and locks an 11.92 fee: 9.54 to the partner, 2.38 to the protocol. |
| 8. Bob exits | Bob redeems all his shares for 3,084.94 TUSDC: +84.94 over both periods. |

Totals: the depositors received 146.01, the partner 29.40, and the infrastructure provider 7.35.

Between steps the LP account arbitrages the pool toward NAV (`demo.sh arb`). A withdrawal that
needs to sell TBOND is bounded by `slippage_bps` around the oracle rate, so it reverts while the
pool trades below NAV by more than that. This is the protection working as designed.

## Try it yourself

The deployment stays live after the scripted scenario. With any testnet wallet:

1. Fund the account with testnet XLM from [Friendbot](https://friendbot.stellar.org).
2. Claim 10,000 TUSDC and 1,000 TBOND from the faucet (`claim(to)`, once an hour per address),
   from the dApp's "Get test tokens" card or the CLI.
3. Deposit TUSDC into the vault, watch the share value follow the NAV, withdraw.

`testnet/keeper.sh loop` keeps it moving every 30 minutes:
- It grows the NAV at 5% a year and pushes it to the oracle.
- It moves idle vault USDC into the strategy and has the keeper buy TBOND, keeping 5,000 TUSDC
  idle in both the vault and the strategy for withdrawals.
- It arbitrages the pool back within 50 bps of NAV.
- Every 6 hours it runs the revenue share (report, lock fees, distribute).

Admin, manager and keeper calls stay with the deployment's keys. OTC is keeper-only and a
counterparty is allowlisted only after a 72-hour timelock: the LP account was proposed on
2026-11-06 and becomes executable at 2026-11-09 10:59 UTC.

## Reproduce

```bash
cd testnet
for k in admin keeper issuer lp partner alice bob; do stellar keys generate ts-$k --network testnet --fund; done
./setup.sh                                # tokens, oracle, pool, strategy, vault -> ../testnet.contracts.json
./add_liquidity.sh 10000000000000         # deeper pool for 5k tranches
./demo.sh fund ts-alice 10000 && ./demo.sh fund ts-bob 10000
./demo.sh deposit ts-alice 5000 && ./demo.sh deposit ts-bob 3000
./demo.sh rebalance 6000 && ./demo.sh invest 5000
./demo.sh nav 300 && ./demo.sh fees
./demo.sh arb 13000 && ./demo.sh withdraw ts-alice all
./demo.sh nav 200 && ./demo.sh fees
./demo.sh arb 15000 && ./demo.sh withdraw ts-bob all
./demo.sh status
./keeper.sh loop                          # keep it live: NAV, pool, allocation, revenue share
```

Reflector stops answering a price one resolution (1 h) after the last push, so run
`./push_nav.sh` at least hourly while the demo is live. Otherwise the strategy goes to
`OracleDown`, which refuses deposits and keeper buys while RWA is held.
