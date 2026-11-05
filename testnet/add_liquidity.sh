#!/usr/bin/env bash
# Mints test tokens to the LP and adds full-range liquidity to the pool at the current price.
# Usage: ./add_liquidity.sh <liquidity_units>    (1e12 is ~100k of each token at 1.0)
source "$(dirname "${BASH_SOURCE[0]}")/_env.sh"
POOL="$(state_get pool)"; LP="$(addr "$LP_ID")"; L="${1:?liquidity}"
inv "$(state_get usdc)" "$ADMIN_ID" -- mint --to "$LP" --amount "$((L * 2))" >/dev/null
inv "$(state_get rwa)" "$ADMIN_ID" -- mint --to "$LP" --amount "$((L * 2))" >/dev/null
hints="$(view "$POOL" -- get_oracle_hints)"
inv "$POOL" "$LP_ID" -- mint --sender "$LP" --recipient "$LP" --tick_lower -887270 --tick_upper 887270 --amount "$L" --hints "$hints"
