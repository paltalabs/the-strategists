#!/usr/bin/env bash
#
# Propose an OTC counterparty on a deployed RWA yield-sharing strategy.
#
# Adding trust waits: the address becomes allowlisted only after ./execute_otc_counterparty.sh
# runs TIMELOCK_SECONDS (72h) later. `OtcCounterpartyProposedEvent` is the public notice the bot
# and the vault manager watch for inside the window — a proposal the operator
# did not expect is the signal to pause the strategy at the vault, not to execute it.
#
# Usage:  DEPLOYMENT=<ustry|usdy|dejtrsy> ./propose_otc_counterparty.sh <network> <admin_identity> <counterparty>

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "PROPOSE OTC COUNTERPARTY"
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
echo "⚠  This starts a public 72h wait; the counterparty is only tradeable after execute_otc_counterparty.sh."
read -rp "Proceed? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

AT="$(stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- propose_otc_counterparty --counterparty "$CP" | tr -d '"')"

DATE="$(date -u -r "$AT" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u -d "@$AT" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || echo "$AT")"
echo "✅ proposed — executable_at = $AT ($DATE)"
