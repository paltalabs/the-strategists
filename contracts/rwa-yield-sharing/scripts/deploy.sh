#!/usr/bin/env bash
#
# Deploy a RWA yield-sharing strategy instance.
#
# A preset pins the asset, venue and oracle set for one of the three planned deployments
# (USTRY, USDY, deJTRSY). `custom` leaves RWA_ASSET / VENUE / ORACLES to be
# supplied as pre-built SCVal JSON via environment variables, for anything not in the catalog.
# Caps default to the depth measured at design time and the largest <=1% impact size for
# deJTRSY; re-derive them at deploy time from live pool depth, don't trust these blindly.
#
# ⚠  MAINNET-ONLY IN PRACTICE. `__constructor` makes live cross-contract reads before storing
#    anything: `decimals()` on the asset and the RWA, `decimals()` on every oracle source, and
#    a token/pool check on the venue (`get_tokens` for Aqua, `token0`/`token1` for Sushi).
#    There is no Aqua or Sushi deployment on testnet, so a testnet deploy has nothing real to
#    point at and will fail at construction. That failure is by design: it aborts a bad deploy
#    before any state is persisted, rather than leaving the strategy live with an invalid Config.
#
# Usage:
#   Interactive:  ./deploy.sh
#   Positional:   ./deploy.sh <network> <source_account> <preset: ustry|usdy|dejtrsy|custom> \
#                             <admin> <keeper> \
#                             [max_swap] [max_invested] [max_spread_bps] [slippage_bps]
#
# Every preset prices off RedStone alone. RedStone publishes the issuer's NAV; Reflector's
# Stellar feed (CALI2BYU…PLE6M) is a VWAP of Stellar DEX trades, i.e. the pool price, which sat
# 135-150 bps (USDY) and ~120 bps (deJTRSY) under NAV through 2026 and printed 107 USDC for USTRY in the
# YieldBlox incident. max_spread_bps defaults to 10, the contract minimum: the most
# NAV can move per hour (~2.4%/day compounded). RedStone's largest single NAV write in 90 days was
# 4 bps. slippage_bps defaults to 0: a withdraw shortfall sells into the pool only at NAV or
# better, so an exit never costs the holders who stay; with the pools under NAV that shortfall
# reverts and the keeper refills idle with divest_otc instead.
#
# The deployer needs only XLM for fees — the constructor calls no require_auth, so admin and
# keeper may be any address and need not sign here.
#
# `custom` reads RWA_ASSET / VENUE / ORACLES from the environment (pre-built SCVal JSON — see
# the aqua_venue / sushi_venue / sep40_source / redstone_source helpers below and in _common.sh):
#   RWA_ASSET=C... VENUE='{"vec":[...]}' ORACLES='{"vec":[...]}' ./deploy.sh mainnet me custom ...

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

readonly WASM_PATH_DEFAULT="$WORKSPACE_ROOT/target/wasm32v1-none/release/rwa_yield_sharing_strategy.wasm"

# --- Fixed underlying and shared oracle constants ---
readonly USDC="CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"
# 24h = MAX_TTL_SECONDS, 2x the documented 12h heartbeat (app.redstone.finance/push-feeds)
readonly REDSTONE_TTL=86400

# --- Venue SCVal JSON builders (sep40_source / redstone_source live in _common.sh — shared
# with propose_oracles.sh). Only deploy.sh builds a Venue: it has no setter, so no
# other script needs these. ---
aqua_venue() { # pool rwa_idx usdc_idx
  printf '{"vec":[{"symbol":"Aqua"},{"address":"%s"},{"u32":%s},{"u32":%s}]}' "$1" "$2" "$3"
}
sushi_venue() { # pool rwa_is_token0
  printf '{"vec":[{"symbol":"Sushi"},{"address":"%s"},{"bool":%s}]}' "$1" "$2"
}
# The Aqua token indices are the pool's own order, so READ it rather than guess it: strkeys are
# base32 (A-Z then 2-7), which is not ASCII-monotonic, so the old `[[ $rwa < $USDC ]]` compare
# could invert the pair. A wrong pair is caught only by `validate_venue` inside the constructor
# — a failed deploy, never a bad deployment — but there is no reason to spend a mainnet
# simulation on it. Read-only invoke (`--send no`); fails closed when the pool does not answer
# with exactly the {rwa, USDC} pair.
aqua_indices() { # pool rwa -> "rwa_idx usdc_idx"
  local tokens t0 t1
  tokens="$(stellar contract invoke --id "$1" --source-account "$SOURCE_ACCOUNT" \
    --network "$NETWORK" --send no -- get_tokens 2>/dev/null | tr -d '[:space:]' || true)"
  t0="$(printf '%s' "$tokens" | sed -n 's/^\["\([^"]*\)","\([^"]*\)"\]$/\1/p')"
  t1="$(printf '%s' "$tokens" | sed -n 's/^\["\([^"]*\)","\([^"]*\)"\]$/\2/p')"
  if [[ "$t0" == "$2" && "$t1" == "$USDC" ]]; then echo "0 1"; return; fi
  if [[ "$t1" == "$2" && "$t0" == "$USDC" ]]; then echo "1 0"; return; fi
  die "pool $1: get_tokens returned ${tokens:-<no answer>}, not the {$2, USDC} pair this preset expects"
}

banner "DEPLOY"

NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"

SOURCE_ACCOUNT=$(resolve_required "Deployer identity (pays fees)" "${2-__UNSET__}")

ensure_network
ensure_signer
echo

PRESET=$(resolve_required "preset (ustry|usdy|dejtrsy|custom)" "${3-__UNSET__}")
ADMIN=$(resolve_required "admin — bounded knobs + the three timelocked proposals" "${4-__UNSET__}")
KEEPER=$(resolve_required "keeper — invest/divest/OTC fills/harvest/pause" "${5-__UNSET__}")

ASSET="$USDC"

case "$PRESET" in
  ustry)
    RWA_ASSET="CBLV4ATSIWU67CFSQU2NVRKINQIKUZ2ODSZBUJTJ43VJVRSBTZYOPNUR"
    AQUA_POOL="CCX2TYR4AQTPPMTZOIMP3YIBHAYLLSTXVP47PAGTL753PCQFSKV32MIA"
    INDICES="$(aqua_indices "$AQUA_POOL" "$RWA_ASSET")" || exit 1
    read -r RI UI <<< "$INDICES"
    VENUE=$(aqua_venue "$AQUA_POOL" "$RI" "$UI")
    ORACLES=$(printf '{"vec":[%s]}' "$(redstone_source "CAEGYZFAJ3HLSDBUIVJ77SK5HLLS4MPE4LO5OB2W3L2TEDMAS6RCTMZ4" "$REDSTONE_TTL")")
    DEFAULT_MAX_SWAP=100000000000        # 10k USDC: ~1% impact on the 1.1M pool
    DEFAULT_MAX_INVESTED=60000000000000  # 6M USDC
    DEFAULT_MAX_SPREAD_BPS=10
    ;;
  usdy)
    RWA_ASSET="CB3YA656OYIHU57657I5KGSBRHE5I3OZU4VFC22PYAOANFZHEWNYGAGP"
    AQUA_POOL="CAFHLHGZXOVNCGFJ7DOXL7JDNMBCEZKDI3LS5NRQH3GXC7CSIMQZHUSM"
    INDICES="$(aqua_indices "$AQUA_POOL" "$RWA_ASSET")" || exit 1
    read -r RI UI <<< "$INDICES"
    VENUE=$(aqua_venue "$AQUA_POOL" "$RI" "$UI")
    ORACLES=$(printf '{"vec":[%s]}' "$(redstone_source "CAEUQPFSHHZ2EH4RKMU6S2NEOZVMDMZZ47RRN5FZCAPDME5OACBHEOXM" "$REDSTONE_TTL")")
    DEFAULT_MAX_SWAP=100000000000
    DEFAULT_MAX_INVESTED=60000000000000
    DEFAULT_MAX_SPREAD_BPS=10
    ;;
  dejtrsy)
    RWA_ASSET="CBI7UCH5KGSVQRO5H4SUCZUTZABCITZLRHQQZTWL2TK4RZ72TAR6IHRV"   # 18 dp
    VENUE=$(sushi_venue "CDMJRRH5MAJLB7T5SQAWQOOL7UJ3BABGTURSVKIE6AEDSXESDDJIBVCT" true)  # token0 = deJTRSY
    ORACLES=$(printf '{"vec":[%s]}' "$(redstone_source "CBOERMDX6PS5QYPBCCTV26JUJIVPQ2MCODQGQFOWVVGFJZLARZCBWTDK" "$REDSTONE_TTL")")
    DEFAULT_MAX_SWAP=1000000000000       # 100k USDC: largest size at <=1% impact
    DEFAULT_MAX_INVESTED=80000000000000  # 8M USDC
    DEFAULT_MAX_SPREAD_BPS=10
    ;;
  custom)
    RWA_ASSET=$(resolve_required "rwa_asset" "${RWA_ASSET-__UNSET__}")
    VENUE=$(resolve_required "venue (SCVal JSON, see aqua_venue/sushi_venue)" "${VENUE-__UNSET__}")
    ORACLES=$(resolve_required "oracles (SCVal JSON vec of sources)" "${ORACLES-__UNSET__}")
    DEFAULT_MAX_SWAP=100000000000; DEFAULT_MAX_INVESTED=10000000000000; DEFAULT_MAX_SPREAD_BPS=10
    ;;
  *) die "preset must be ustry|usdy|dejtrsy|custom" ;;
esac
readonly DEFAULT_SLIPPAGE_BPS=0

MAX_SWAP=$(resolve_with_default "max_swap (stroops)" "$DEFAULT_MAX_SWAP" "${6-__UNSET__}")
MAX_INVESTED=$(resolve_with_default "max_invested (stroops)" "$DEFAULT_MAX_INVESTED" "${7-__UNSET__}")
MAX_SPREAD_BPS=$(resolve_with_default "max_spread_bps" "$DEFAULT_MAX_SPREAD_BPS" "${8-__UNSET__}")
SLIPPAGE_BPS=$(resolve_with_default "slippage_bps" "$DEFAULT_SLIPPAGE_BPS" "${9-__UNSET__}")

WASM_PATH="${WASM_PATH:-$WASM_PATH_DEFAULT}"
INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"

# --- Validation: fail here, not 30s into a mainnet simulation ---

[[ -f "$WASM_PATH" ]] || die "wasm not found at '$WASM_PATH'. Build it first:
    stellar contract build
  (Set WASM_PATH=… to deploy a different artifact, e.g. the .optimized.wasm.)"

[[ "$ASSET" != "$RWA_ASSET" ]] || die "asset must differ from rwa_asset (the constructor asserts this)"

require_positive_i128 "max_swap" "$MAX_SWAP"
require_positive_i128 "max_invested" "$MAX_INVESTED"
require_positive_i128 "inclusion fee" "$INCLUSION_FEE"
# slippage_bps may legitimately be 0 (no tolerance), so it isn't require_positive_i128.
[[ "$SLIPPAGE_BPS" =~ ^[0-9]+$ ]] || die "slippage_bps must be a non-negative integer, got '$SLIPPAGE_BPS'"

# Mirror the constructor's bounds (constants.rs MIN_SPREAD_BPS/MAX_SPREAD_BPS/MAX_SLIPPAGE_BPS)
# so a bad value is caught before we pay to find out.
(( MAX_SPREAD_BPS >= 10 && MAX_SPREAD_BPS <= 500 )) \
  || die "max_spread_bps out of range [10, 500], got '$MAX_SPREAD_BPS'"
(( SLIPPAGE_BPS <= 500 )) \
  || die "slippage_bps out of range [0, 500] (constants.rs MAX_SLIPPAGE_BPS), got '$SLIPPAGE_BPS'"

# --- init_args ---
#
# init_args is a Vec<Val>; the CLI wants each element as a tagged SCVal-JSON object.
# Order mirrors lib.rs __constructor (init_args[0..8]):
#   0 rwa_asset (Address)  1 admin (Address)  2 keeper (Address)  3 oracles (Vec<OracleSource>)
#   4 venue (Venue)  5 max_swap (i128)  6 max_invested (i128)  7 max_spread_bps (u32)
#   8 slippage_bps (u32)

INIT_ARGS=$(printf '[{"address":"%s"},{"address":"%s"},{"address":"%s"},%s,%s,{"i128":"%s"},{"i128":"%s"},{"u32":%s},{"u32":%s}]' \
  "$RWA_ASSET" "$ADMIN" "$KEEPER" "$ORACLES" "$VENUE" \
  "$MAX_SWAP" "$MAX_INVESTED" "$MAX_SPREAD_BPS" "$SLIPPAGE_BPS")

# Round-trip every element through the encoder so a malformed JSON fails locally, not 30s into
# a mainnet simulation.
if command -v jq >/dev/null 2>&1; then
  while IFS= read -r elem; do
    printf '%s' "$elem" | stellar xdr encode --type ScVal --input json >/dev/null \
      || die "init_args element failed to encode as ScVal: $elem"
  done < <(printf '%s' "$INIT_ARGS" | jq -c '.[]')
fi

DEPLOY_KEY="rwa-yield-sharing-$PRESET"

echo
echo "──────────────────────────────────────"
echo " network:         $NETWORK"
echo " source:          $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " wasm:            $WASM_PATH"
echo " preset:          $PRESET"
echo " asset:           $ASSET (USDC)"
echo " rwa_asset:       $RWA_ASSET"
echo " admin:           $ADMIN"
echo " keeper:          $KEEPER"
echo " venue:           $VENUE"
echo " oracles:         $ORACLES"
echo " max_swap:        $MAX_SWAP stroops"
echo " max_invested:    $MAX_INVESTED stroops"
echo " max_spread_bps:  $MAX_SPREAD_BPS"
echo " slippage_bps:    $SLIPPAGE_BPS"
echo " inclusion fee:   $INCLUSION_FEE stroops"
echo "──────────────────────────────────────"
echo "⚠  The venue has no setter — a pool migration is a new deployment. Admin and keeper are"
echo "   rotatable, but the RWA, venue and USDC asset are IMMUTABLE for the life of this instance."

read -rp "Deploy now? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

DEPLOYED_ADDRESS="$(stellar contract deploy \
  --wasm "$WASM_PATH" \
  --source-account "$SOURCE_ACCOUNT" \
  --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- \
  --asset "$ASSET" \
  --init_args "$INIT_ARGS")"

echo
echo "✅ Deployed RWA yield-sharing strategy ($PRESET): $DEPLOYED_ADDRESS"
echo
echo "⚠  NEXT STEPS:"
echo
echo "  1. Allowlist the DeFindex vault as ADMIN, by address or by wasm code hash:"
echo
echo "     DEPLOYMENT=$PRESET ./add_depositor.sh $NETWORK <admin_identity> <vault_address>"
echo "     DEPLOYMENT=$PRESET ./add_allowed_hash.sh $NETWORK <admin_identity> <hash_or_path_to.wasm>"
echo
echo "  2. Propose every OTC counterparty you expect to use TODAY so the 72h wait"
echo "     ends before the vault moves funds, then execute once it does:"
echo
echo "     DEPLOYMENT=$PRESET ./propose_otc_counterparty.sh $NETWORK <admin_identity> <counterparty>"
echo "     DEPLOYMENT=$PRESET ./execute_otc_counterparty.sh $NETWORK <admin_identity> <counterparty>"
echo
echo "  3. Leave auto_invest OFF, during and after migration — deposits stay idle at par, and the"
echo "     keeper invests idle back toward the target buffer on a schedule:"
echo
echo "     DEPLOYMENT=$PRESET ./invest_otc.sh ...   (or ./invest.sh ...; never set_auto_invest true)"
echo
echo "  4. Allowlist the keeper's emergency stop: the keeper can freeze the price at the anchor"
echo "     at once (./freeze_price.sh), only the admin can lift it. Wire the bot to freeze when"
echo "     RedStone moves more than a few bps against the issuer's published NAV."
echo
echo "  5. The keeper's TTL-extension duty: extend the instance TTL of every"
echo "     configured oracle and venue contract (stellar contract extend) as part of its loop —"
echo "     an archived (TTL-expired) instance is a ledger-level failure no contract code can catch."
echo
echo "  6. Record the address in $WORKSPACE_ROOT/$NETWORK.contracts.json under"
echo "     \"$DEPLOY_KEY\" so the other scripts resolve it automatically:"
echo
echo "     jq '. + {\"$DEPLOY_KEY\":\"$DEPLOYED_ADDRESS\"}' $NETWORK.contracts.json | sponge $NETWORK.contracts.json"
