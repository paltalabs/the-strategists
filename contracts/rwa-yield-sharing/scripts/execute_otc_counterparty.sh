#!/usr/bin/env bash
#
# Execute a proposed OTC counterparty on a deployed RWA yield-sharing strategy, once its
# 72h wait has passed. Allowlists the address; nothing before then (the contract itself
# refuses with NotAuthorized before executable_at — this is a local heads-up, not a bypass).
#
# Usage:  DEPLOYMENT=<ustry|usdy|dejtrsy> ./execute_otc_counterparty.sh <network> <admin_identity> <counterparty>

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "EXECUTE OTC COUNTERPARTY"
NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"
SOURCE_ACCOUNT=$(resolve_required "Signer identity (must be the admin)" "${2-__UNSET__}")
ensure_network
ensure_signer
load_strategy_id
echo
CP=$(resolve_required "Counterparty address" "${3-__UNSET__}")
[[ "$CP" =~ ^[GC][A-Z2-7]{55}$ ]] \
  || die "counterparty must look like a Stellar address (G... account or C... contract), got '$CP'"

PENDING="$(stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" --send no \
  -- get_pending_otc_counterparty --counterparty "$CP" 2>/dev/null | tr -d '"' || true)"
[[ -n "$PENDING" && "$PENDING" != "null" ]] \
  || die "no pending proposal for '$CP' (get_pending_otc_counterparty returned none). Run propose_otc_counterparty.sh first."

PENDING_DATE="$(date -u -r "$PENDING" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u -d "@$PENDING" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || echo "$PENDING")"
echo "ℹ pending proposal for $CP: executable_at = $PENDING ($PENDING_DATE)"

NOW="$(date -u +%s)"
if (( NOW < PENDING )); then
  die "not executable yet — $((PENDING - NOW))s remain until $PENDING_DATE"
fi

INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"

echo "──────────────────────────────────────"
echo " strategy:     $STRATEGY_ID ($STRATEGY_KEY)"
echo " signer:       $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " counterparty: $CP"
echo "──────────────────────────────────────"
read -rp "Proceed? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- execute_otc_counterparty --counterparty "$CP"
echo "✅ $CP is now an allowlisted OTC counterparty"
