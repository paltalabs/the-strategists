#!/usr/bin/env bash
#
# Toggle auto_invest on a deployed RWA yield-sharing strategy.
#
# When ON, every deposit that lands while pricing is Live buys one best-effort tranche of at
# most max_swap on arrival. Leave it OFF in normal operation: deposits land as idle USDC at
# par with no swap, and the keeper invests idle back
# toward the target buffer on a schedule with invest_otc.sh / invest.sh. Kept for an emergency
# or a later decision, not for normal operation.
#
# Usage:  DEPLOYMENT=<ustry|usdy|dejtrsy> ./set_auto_invest.sh <network> <admin_identity> <true|false>

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "SET AUTO INVEST"
NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"
SOURCE_ACCOUNT=$(resolve_required "Signer identity (must be the admin)" "${2-__UNSET__}")
ensure_network
ensure_signer
load_strategy_id
echo
ENABLED=$(resolve_required "enabled (true = buy on deposit, false = idle only)" "${3-__UNSET__}")
[[ "$ENABLED" == "true" || "$ENABLED" == "false" ]] \
  || die "expected 'true' or 'false', got '$ENABLED'"
INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"

echo "──────────────────────────────────────"
echo " strategy:     $STRATEGY_ID ($STRATEGY_KEY)"
echo " signer:       $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " auto_invest:  $ENABLED"
echo "──────────────────────────────────────"
read -rp "Proceed? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- set_auto_invest --enabled "$ENABLED"
echo "✅ auto_invest = $ENABLED"
