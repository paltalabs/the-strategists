#!/usr/bin/env bash
#
# Propose a new oracle set on a deployed RWA yield-sharing strategy. Adding trust waits: the
# set takes effect only after ./execute_oracles.sh runs TIMELOCK_SECONDS (72h) later AND one of
# its sources reads live at that time (src/oracles.rs:92-99; no distance-to-anchor check). To hold NAV while the wait runs, freeze
# the price (./freeze_price.sh). `OraclesProposedEvent { count, executable_at }` is the public
# notice — it carries only a count, so read the proposal itself for the actual feeds, not the
# event alone. There is no get_pending_oracles.sh; the read is one invoke:
#
#   stellar contract invoke --id <strategy_id> --source-account <any_identity> \
#     --network <network> --send no -- get_pending_oracles
#
# which is exactly what ./execute_oracles.sh (line 22-24) and ./cancel_oracles.sh (line 20-22)
# run inline before they submit.
#
# Every source prices THIS instance's rwa_asset (read from get_config — one deployed instance
# is always one RWA), so you supply only the feed/adapter address, its TTL and (for Sep40) its
# decimals:
#
#   --sep40 <feed> <ttl_seconds> <decimals>   Reflector-shaped SEP-40 feed
#   --redstone <adapter> <ttl_seconds>        RedStone per-asset adapter (8 decimals)
#
# Repeat either flag up to MAX_ORACLES (3) times, in the order you want them evaluated.
#
# Usage:
#   DEPLOYMENT=<ustry|usdy|dejtrsy> ./propose_oracles.sh <network> <admin_identity> \
#     [--sep40 <feed> <ttl_seconds> <decimals>] [--redstone <adapter> <ttl_seconds>] ...

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "PROPOSE ORACLES"
NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"
SOURCE_ACCOUNT=$(resolve_required "Signer identity (must be the admin)" "${2-__UNSET__}")
ensure_network
ensure_signer
load_strategy_id
echo
shift 2 2>/dev/null || true

RWA_ASSET="$(stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" --send no \
  -- get_config 2>/dev/null | (jq -r '.rwa_asset' 2>/dev/null || true) || true)"
[[ -n "$RWA_ASSET" && "$RWA_ASSET" != "null" ]] || die "failed to read rwa_asset from get_config"
echo "ℹ rwa_asset (from get_config): $RWA_ASSET"

SOURCES=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --sep40)
      [[ $# -ge 4 ]] || die "--sep40 needs <feed> <ttl_seconds> <decimals>"
      SOURCES+=("$(sep40_source "$2" "$RWA_ASSET" "$3" "$4")")
      shift 4
      ;;
    --redstone)
      [[ $# -ge 3 ]] || die "--redstone needs <adapter> <ttl_seconds>"
      SOURCES+=("$(redstone_source "$2" "$3")")
      shift 3
      ;;
    *)
      die "unknown flag '$1' (expected --sep40 <feed> <ttl_seconds> <decimals> or --redstone <adapter> <ttl_seconds>)"
      ;;
  esac
done

(( ${#SOURCES[@]} > 0 && ${#SOURCES[@]} <= 3 )) \
  || die "need 1..3 oracle sources (MAX_ORACLES=3), got ${#SOURCES[@]}"

ORACLES_JSON=$(IFS=,; printf '{"vec":[%s]}' "${SOURCES[*]}")

# Round-trip every source through the encoder so a malformed JSON fails locally.
if command -v jq >/dev/null 2>&1; then
  while IFS= read -r elem; do
    printf '%s' "$elem" | stellar xdr encode --type ScVal --input json >/dev/null \
      || die "oracle source failed to encode as ScVal: $elem"
  done < <(printf '%s' "$ORACLES_JSON" | jq -c '.vec[]')
fi

INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"

echo
echo "──────────────────────────────────────"
echo " strategy:  $STRATEGY_ID ($STRATEGY_KEY)"
echo " signer:    $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " sources:   ${#SOURCES[@]}"
echo " oracles:   $ORACLES_JSON"
echo "──────────────────────────────────────"
echo "⚠  This starts a public 72h wait; the set takes effect only via execute_oracles.sh, and"
echo "   only if at least one proposed source reads live then."
read -rp "Proceed? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

AT="$(stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- propose_oracles --oracles "$ORACLES_JSON" | tr -d '"')"

DATE="$(date -u -r "$AT" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u -d "@$AT" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || echo "$AT")"
echo "✅ proposed — executable_at = $AT ($DATE)"
