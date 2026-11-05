# Vendored contract WASMs (test fixtures)

Pinned so the integration tests are self-contained and reproducible.

| File | Source |
|------|--------|
| soroban_liquidity_pool_router_contract.wasm | Aquarius AMM `liquidity_pool_router` |
| soroban_liquidity_pool_contract.wasm | Aquarius AMM `liquidity_pool` (constant-product) |
| soroban_liquidity_pool_plane_contract.wasm | Aquarius AMM `liquidity_pool_plane` |
| soroban_liquidity_pool_liquidity_calculator_contract.wasm | Aquarius AMM `liquidity_pool_liquidity_calculator` |
| soroban_locker_feed_contract.wasm | Aquarius AMM `locker_feed` |
| soroban_rewards_gauge_contract.wasm | Aquarius AMM `rewards_gauge` |
| soroban_config_storage_contract.wasm | Aquarius AMM `liquidity_pool_config_storage` |
| soroban_token_contract.wasm | Aquarius AMM `token` (LP share) |
| reflector_oracle.wasm | Reflector SEP-40 oracle |
| sushi_pool.wasm | Sushi deJTRSY/USDC pool `CDMJRRH5MAJLB7T5SQAWQOOL7UJ3BABGTURSVKIE6AEDSXESDDJIBVCT`, fetched from mainnet 2026-09-23, sha256 `003710b383f9da7d650a7f719a7be479110266427817ebbed61d924505fcd7c7` |
| redstone_price_adapter.wasm | RedStone USTRY price adapter `CAEGYZFAJ3HLSDBUIVJ77SK5HLLS4MPE4LO5OB2W3L2TEDMAS6RCTMZ4`, fetched from mainnet 2026-09-23, sha256 `3e464b6dda26a8d5b0bf13d2cb7781a6142bf2c2e4b64edcba093911fa16df5c`. Its `read_price_data` calls the RedStone router, absent in tests, so only `decimals()` and the failure path are exercised |
