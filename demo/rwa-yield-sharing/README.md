# RWA Yield-Sharing: testnet demo

> Status: to build.

Scenario the scripts run on testnet, recording every transaction hash in `run-log.md`:

1. Setup: mock RWA token, mock SEP-40 oracle (controllable NAV), Aqua or Sushi pool with liquidity, strategy, DeFindex vault (factory `CDSCWE4GLNBYYTES2OCYDFQA2LLY4RBIAX6ZI32VSUXD7GO6HRPO4A32`) with a configured fee split
2. Deposits: two users deposit USDC into the vault; the keeper invests into the RWA
3. Yield accrual: raise the oracle NAV in steps over time
4. Profit attribution: each user's balance grows in proportion to their shares
5. Revenue sharing: the vault reports and distributes fees; the partner and infrastructure receivers get their share; users withdraw principal plus their net yield
