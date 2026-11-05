#!/usr/bin/env bash
#
# Pause / unpause new deposits on a deployed RWA yield-sharing strategy.
#
# WITHDRAWALS ARE NEVER PAUSED — pausing only refuses new capital (deposit, invest, invest_otc),
# so vaults can always exit. This is the lever to pull when the RWA's peg, its venue, or its
# oracle feed looks wrong and you want to stop taking on more exposure while you investigate —
# the first step of the emergency procedure.
#
# Reads the strategy address from <workspace_root>/<network>.contracts.json
# (key "rwa-yield-sharing[-<DEPLOYMENT>]"); prompts if the file/key is missing.
# Honors DEPLOYMENT=<ustry|usdy|dejtrsy> (unset = the default instance).
#
# Usage:
#   Interactive:  ./pause_deposits.sh
#   Positional:   ./pause_deposits.sh <network> <source_account> <true|false>
#
# The signer must be the ADMIN or the KEEPER (pause_deposits accepts either).

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "PAUSE DEPOSITS"

NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"

SOURCE_ACCOUNT=$(resolve_required "Signer identity (must be the admin or the keeper)" "${2-__UNSET__}")

ensure_network
ensure_signer
load_strategy_id
echo

PAUSED=$(resolve_required "Paused (true = refuse new deposits, false = resume)" "${3-__UNSET__}")
[[ "$PAUSED" == "true" || "$PAUSED" == "false" ]] \
  || die "expected 'true' or 'false', got '$PAUSED'"

INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"
require_positive_i128 "inclusion fee" "$INCLUSION_FEE"

echo
echo "──────────────────────────────────────"
echo " network:    $NETWORK"
echo " signer:     $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " strategy:   $STRATEGY_ID ($STRATEGY_KEY)"
echo " caller:     $SIGNER_PUBKEY"
echo " paused:     $PAUSED"
echo " inclusion fee:  $INCLUSION_FEE stroops"
echo "──────────────────────────────────────"
if [[ "$PAUSED" == "true" ]]; then
  echo "→ new deposits/invest/invest_otc will be REJECTED. Withdrawals stay open."
else
  echo "→ deposits will be ACCEPTED again."
fi

read -rp "Proceed? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" \
  --source-account "$SOURCE_ACCOUNT" \
  --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- pause_deposits \
  --caller "$SIGNER_PUBKEY" \
  --paused "$PAUSED"

echo
echo "✅ paused = $PAUSED"
