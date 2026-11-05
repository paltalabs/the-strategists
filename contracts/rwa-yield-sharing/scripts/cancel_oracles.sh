#!/usr/bin/env bash
#
# Cancel the pending oracle-set proposal on a deployed RWA yield-sharing strategy —
# immediate, no wait. Refuses on-chain when there is no pending proposal.
#
# Usage:  DEPLOYMENT=<ustry|usdy|dejtrsy> ./cancel_oracles.sh <network> <admin_identity>

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "CANCEL ORACLES"
NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"
SOURCE_ACCOUNT=$(resolve_required "Signer identity (must be the admin)" "${2-__UNSET__}")
ensure_network
ensure_signer
load_strategy_id
echo

PENDING="$(stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" --send no \
  -- get_pending_oracles 2>/dev/null || true)"
if [[ -z "$PENDING" || "$PENDING" == "null" ]]; then
  die "no pending oracle proposal on this strategy (get_pending_oracles returned none)."
fi
echo "ℹ pending proposal: $PENDING"

INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"

echo "──────────────────────────────────────"
echo " strategy:  $STRATEGY_ID ($STRATEGY_KEY)"
echo " signer:    $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo "──────────────────────────────────────"
read -rp "Cancel this proposal now? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- cancel_oracles
echo "✅ pending oracle proposal cancelled"
