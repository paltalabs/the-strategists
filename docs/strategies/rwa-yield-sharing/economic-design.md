# RWA Yield-Sharing Strategy: economic design

> Status: outline. To be written for Tranche 1.

1. Compatible yield-bearing assets: Etherfuse (CETES, USTRY), Ondo (USDY), Centrifuge (deJTRSY); issuer, NAV source, liquidity on Stellar
2. How yield accrues: price appreciation (NAV), not balance growth
3. Why USDC in / USDC out: a value-accruing token never grows its balance, so balance-based fee accounting sees zero yield; holding the RWA and settling in USDC turns accrual into measurable base-asset growth
4. Measuring profit: oracle NAV times holdings, never the pool price; pools trade 100-150 bps under NAV
5. Revenue sharing: vault fee split (user / partner app / infrastructure provider), worked example
6. Liquidity: idle buffer, pool tranches, OTC at NAV; cost of exits
7. Baskets: several strategies in one vault to diversify issuers
