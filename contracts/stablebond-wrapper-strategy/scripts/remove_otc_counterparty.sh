#!/usr/bin/env bash
#
# Remove an OTC counterparty from a deployed RWA yield-sharing strategy — immediate, no wait.
#
# Removing trust never waits: this clears the allowlist entry AND any pending
# proposal for the address in the same transaction. Never affects invest/divest/withdraw
# through the venue — only OTC fills with this counterparty stop.
#
# Usage:  DEPLOYMENT=<ustry|usdy|dejtrsy> ./remove_otc_counterparty.sh <network> <admin_identity> <counterparty>

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "REMOVE OTC COUNTERPARTY"
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
INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"

echo "──────────────────────────────────────"
echo " strategy:     $STRATEGY_ID ($STRATEGY_KEY)"
echo " signer:       $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " counterparty: $CP"
echo "──────────────────────────────────────"
echo "→ takes effect immediately; also clears any pending proposal for this address."
read -rp "Remove this counterparty now? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- remove_otc_counterparty --counterparty "$CP"
echo "✅ removed $CP"
