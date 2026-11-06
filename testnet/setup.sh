#!/usr/bin/env bash
# Deploys the whole testnet stack, idempotently, writing addresses to testnet.contracts.json:
#   1. two SEP-41 test tokens (TUSDC, TBOND, testnet/test-token), no trustlines needed
#   2. a Reflector SEP-40 oracle quoting TBOND in TUSDC, NAV 1.0
#   3. a Sushi concentrated pool TBOND/TUSDC at 1.0, full-range liquidity
#   4. the strategy (exact production wasm)
#   5. a DeFindex vault from the public testnet factory holding the strategy
#   6. a faucet (testnet/faucet) anyone can claim test TUSDC and TBOND from
# Usage: ./setup.sh
source "$(dirname "${BASH_SOURCE[0]}")/_env.sh"

A="$(addr "$ADMIN_ID")"; K="$(addr "$KEEPER_ID")"; LP="$(addr "$LP_ID")"; P="$(addr "$PARTNER_ID")"
state_set admin "$A"; state_set keeper "$K"; state_set partner "$P"
state_set defindex_receiver "$(view "$DEFINDEX_FACTORY" -- defindex_receiver | tr -d '"')"
state_set defindex_factory "$DEFINDEX_FACTORY"

# --- 1. Tokens -----------------------------------------------------------------
TOKEN_WASM="$HERE/test-token/target/wasm32v1-none/release/test_token.wasm"
deploy_token() { # <key> <name> <symbol>
  local id; id="$(state_get "$1")"
  if [[ -z "$id" ]]; then
    log "deploying token $3"
    [[ -f "$TOKEN_WASM" ]] || (cd "$HERE/test-token" && stellar contract build >/dev/null)
    id="$(retry stellar contract deploy --wasm "$TOKEN_WASM" --source-account "$ADMIN_ID" --network "$NET" -- \
      --admin "$A" --decimals 7 --name "$2" --symbol "$3" | tail -1)"
    state_set "$1" "$id"
  fi
  echo "$id"
}
USDC="$(deploy_token usdc "Test USD Coin" TUSDC)"
RWA="$(deploy_token rwa "Test Treasury Bond" TBOND)"

# --- 2. Oracle -------------------------------------------------------------------
ORACLE="$(state_get oracle)"
if [[ -z "$ORACLE" ]]; then
  log "deploying Reflector"
  ORACLE="$(retry stellar contract deploy --wasm "$EXT/reflector_oracle.wasm" --source-account "$ADMIN_ID" --network "$NET" | tail -1)"
  cfg=$(printf '{"admin":"%s","assets":[{"Stellar":"%s"}],"base_asset":{"Stellar":"%s"},"decimals":%s,"period":86400000,"resolution":%s}' \
    "$A" "$RWA" "$USDC" "$ORACLE_DECIMALS" "$ORACLE_RESOLUTION_MS")
  inv "$ORACLE" "$ADMIN_ID" -- config --config "$cfg" >/dev/null
  state_set oracle "$ORACLE"
  state_set nav "$ORACLE_ONE"
fi
"$HERE/push_nav.sh" "$(state_get nav)"

# --- 3. Sushi pool -------------------------------------------------------------------
POOL="$(state_get pool)"
if [[ -z "$POOL" ]]; then
  FACTORY_STUB="$(state_get sushi_factory)"
  if [[ -z "$FACTORY_STUB" ]]; then
    (cd "$HERE/sushi-factory-stub" && stellar contract build >/dev/null)
    FACTORY_STUB="$(retry stellar contract deploy --wasm "$HERE/sushi-factory-stub/target/wasm32v1-none/release/sushi_factory_stub.wasm" --source-account "$ADMIN_ID" --network "$NET" | tail -1)"
    state_set sushi_factory "$FACTORY_STUB"
  fi
  # token0 must be the lower contract id by bytes, not by strkey string.
  read -r T0 T1 < <(python3 -I -c '
import base64,sys
a,b=sys.argv[1:3]; k=lambda s: base64.b32decode(s)[1:33]
print(*( (a,b) if k(a)<k(b) else (b,a) ))' "$RWA" "$USDC")
  log "deploying Sushi pool (token0 $T0)"
  POOL="$(retry stellar contract deploy --wasm "$EXT/sushi_pool.wasm" --source-account "$ADMIN_ID" --network "$NET" -- \
    --factory "$FACTORY_STUB" --token0 "$T0" --token1 "$T1" --fee 500 --tick_spacing 10 --flash_executor "$A" --admin "$A" | tail -1)"
  inv "$POOL" "$ADMIN_ID" -- initialize --sqrt_price_x96 79228162514264337593543950336 >/dev/null   # 2^96: price 1.0
  state_set pool "$POOL"
  [[ "$T0" == "$RWA" ]] && state_set rwa_is_token0 true || state_set rwa_is_token0 false
fi
if [[ "$(state_get pool_seeded)" != true ]]; then
  LIQ=1000000000000   # ~100k of each token full range at 1.0
  log "minting test tokens to the LP and adding liquidity"
  inv "$USDC" "$ADMIN_ID" -- mint --to "$LP" --amount $((LIQ * 3)) >/dev/null
  inv "$RWA" "$ADMIN_ID" -- mint --to "$LP" --amount $((LIQ * 3)) >/dev/null
  hints="$(view "$POOL" -- get_oracle_hints)"
  inv "$POOL" "$LP_ID" -- mint --sender "$LP" --recipient "$LP" --tick_lower -887270 --tick_upper 887270 --amount "$LIQ" --hints "$hints" >/dev/null
  state_set pool_seeded true
fi

# --- 4. Strategy ---------------------------------------------------------------------
STRATEGY="$(state_get strategy)"
if [[ -z "$STRATEGY" ]]; then
  WASM="$ROOT/target/wasm32v1-none/release/stablebond_wrapper_strategy.wasm"
  (cd "$ROOT" && stellar contract build --package stablebond_wrapper_strategy >/dev/null)
  state_set strategy_wasm_sha256 "$(shasum -a 256 "$WASM" | cut -d' ' -f1)"
  venue=$(printf '{"vec":[{"symbol":"Sushi"},{"address":"%s"},{"bool":%s}]}' "$POOL" "$(state_get rwa_is_token0)")
  oracles=$(printf '{"vec":[{"map":[{"key":{"symbol":"decimals"},"val":{"u32":%s}},{"key":{"symbol":"kind"},"val":{"vec":[{"symbol":"Sep40"},{"address":"%s"},{"address":"%s"}]}},{"key":{"symbol":"ttl_seconds"},"val":{"u64":"%s"}}]}]}' \
    "$ORACLE_DECIMALS" "$ORACLE" "$RWA" "$ORACLE_TTL")
  # max_swap 10k, max_invested 1M, max_spread_bps 500 (the contract maximum, so a NAV step books
  # within one anchor window), slippage_bps 100.
  init=$(printf '[{"address":"%s"},{"address":"%s"},{"address":"%s"},%s,%s,{"i128":"%s"},{"i128":"%s"},{"u32":%s},{"u32":%s}]' \
    "$RWA" "$A" "$K" "$oracles" "$venue" 100000000000 10000000000000 500 100)
  log "deploying the strategy"
  STRATEGY="$(retry stellar contract deploy --wasm "$WASM" --source-account "$ADMIN_ID" --network "$NET" -- --asset "$USDC" --init_args "$init" | tail -1)"
  state_set strategy "$STRATEGY"
fi

# --- 5. Vault ---------------------------------------------------------------------
VAULT="$(state_get vault)"
if [[ -z "$VAULT" ]]; then
  # The vault traps if "DeFindex-Vault-" + name exceeds 32 characters.
  log "creating the DeFindex vault (vault fee 20%, fee receiver = partner)"
  VAULT="$(inv "$DEFINDEX_FACTORY" "$ADMIN_ID" -- create_defindex_vault \
    --roles "{\"0\":\"$A\",\"1\":\"$P\",\"2\":\"$A\",\"3\":\"$A\"}" --vault_fee 2000 \
    --assets "[{\"address\":\"$USDC\",\"strategies\":[{\"address\":\"$STRATEGY\",\"name\":\"RWA Yield-Sharing TBOND\",\"paused\":false}]}]" \
    --soroswap_router "$SOROSWAP_ROUTER" \
    --name_symbol '{"name":"RWA Yield Sharing","symbol":"STRWA"}' --upgradable false | tail -1 | tr -d '"')"
  state_set vault "$VAULT"
fi
if [[ "$(view "$STRATEGY" -- is_depositor --depositor "$VAULT")" != true ]]; then
  log "allowlisting the vault on the strategy"
  inv "$STRATEGY" "$ADMIN_ID" -- add_depositor --depositor "$VAULT" >/dev/null
fi

# --- 6. Faucet ---------------------------------------------------------------------
# The tokens' admin is fixed, so the faucet hands out a pre-minted balance; refill with
# ./demo.sh refill. Per claim: 10,000 TUSDC and 1,000 TBOND, once an hour per address.
FAUCET="$(state_get faucet)"
if [[ -z "$FAUCET" ]]; then
  log "deploying the faucet"
  (cd "$HERE/faucet" && stellar contract build >/dev/null)
  FAUCET="$(retry stellar contract deploy --wasm "$HERE/faucet/target/wasm32v1-none/release/test_faucet.wasm" --source-account "$ADMIN_ID" --network "$NET" -- \
    --tokens "[\"$USDC\",\"$RWA\"]" --amounts '["100000000000","10000000000"]' --cooldown 3600 | tail -1)"
  state_set faucet "$FAUCET"
  inv "$USDC" "$ADMIN_ID" -- mint --to "$FAUCET" --amount 1000000000000000 >/dev/null   # 100M TUSDC
  inv "$RWA" "$ADMIN_ID" -- mint --to "$FAUCET" --amount 100000000000000 >/dev/null    # 10M TBOND
fi

log "done"
jq . "$STATE"
