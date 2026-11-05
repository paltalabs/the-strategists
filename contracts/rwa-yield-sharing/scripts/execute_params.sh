#!/usr/bin/env bash
#
# Execute the pending price-knob proposal on a deployed RWA yield-sharing strategy, once its
# 72h wait has passed. No market check — max_spread_bps, slippage_bps, max_swap and
# otc_max_deviation_bps are bounded constants, not a price, so there is nothing to compare them
# against; the 72h wait is the whole control. Prints ./get_params.sh after so you can confirm
# the four values landed.
#
# Usage:  DEPLOYMENT=<ustry|usdy|dejtrsy> ./execute_params.sh <network> <admin_identity>

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "EXECUTE PARAMS"
NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"
SOURCE_ACCOUNT=$(resolve_required "Signer identity (must be the admin)" "${2-__UNSET__}")
ensure_network
ensure_signer
load_strategy_id
echo

PENDING="$(stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" --send no \
  -- get_pending_params 2>/dev/null || true)"
if [[ -z "$PENDING" || "$PENDING" == "null" ]]; then
  die "no pending params proposal on this strategy (get_pending_params returned none)."
fi
echo "ℹ pending proposal: $PENDING"

AT=""
if command -v jq >/dev/null 2>&1; then
  AT="$(printf '%s' "$PENDING" | jq -r '.executable_at // empty' 2>/dev/null || true)"
fi
if [[ -n "$AT" ]]; then
  AT_DATE="$(date -u -r "$AT" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u -d "@$AT" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || echo "$AT")"
  echo "ℹ executable_at: $AT ($AT_DATE)"
  NOW="$(date -u +%s)"
  if (( NOW < AT )); then
    die "not executable yet — $((AT - NOW))s remain until $AT_DATE"
  fi
fi

INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"

echo "──────────────────────────────────────"
echo " strategy:  $STRATEGY_ID ($STRATEGY_KEY)"
echo " signer:    $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo "──────────────────────────────────────"
read -rp "Execute this proposal now? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- execute_params
echo "✅ price knobs updated"

echo
echo "→ current params:"
"$(dirname "${BASH_SOURCE[0]}")/get_params.sh" "$NETWORK" "$SOURCE_ACCOUNT" || true
