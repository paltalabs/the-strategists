# RWA Yield-Sharing Strategy: threat model and risk analysis

> Status: outline. To be written for Tranche 1.

Per threat: description, impact, mitigation in the contract (with `file:line`), residual risk.

1. Trust model and security assumptions
2. Oracle manipulation or inaccurate feeds (min of sources, anchor clamp, staleness, freeze)
3. Pool price manipulation (pool is never a pricing input; slippage bound around NAV)
4. Accounting inconsistencies in yield attribution (share math, rounding, first-depositor inflation)
5. Timing attacks around deposits and withdrawals (mint at pre-deposit NAV, anchor window, sandwiching tranches)
6. Privileged roles (admin multisig, keeper compromise, timelocks, no caller-chosen recipients)
7. External dependencies (issuer, oracle provider, venue upgradeability)
8. Liquidity risk (withdrawal shortfall, caps)
