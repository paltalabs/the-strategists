# RWA Yield-Sharing Strategy: architecture

> Status: outline. To be written for Tranche 1.

1. Components: DeFindex vault, this strategy, RWA token, venue (Aqua or Sushi pool), oracles (SEP-40 / RedStone), OTC counterparty
2. Roles: vault manager, strategy admin (multisig), keeper (bot), depositors
3. Flows: deposit, invest (pool tranche / OTC), withdraw (idle first, then shortfall sale), harvest
4. Share accounting: virtual-offset shares, rounding toward the strategy
5. Pricing: oracle NAV, anchor clamp, pricing modes (Live / OracleDown / Frozen)
6. Admin surface and timelocks
7. Where revenue sharing happens: the vault's fee split between user, partner and infrastructure provider
8. Interface reference (constructor args, public methods, events)
