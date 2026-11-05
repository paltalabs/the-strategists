#!/usr/bin/env bash
# Pushes the TBOND NAV (USDC per TBOND, 14 decimals) to the Reflector oracle and records it.
# Reflector keeps one price per resolution slot (1 h) and stops answering a slot after it ends,
# so run this at least hourly while the demo is live.
# Usage: ./push_nav.sh [nav_14dp]      default: the last NAV in testnet.contracts.json
source "$(dirname "${BASH_SOURCE[0]}")/_env.sh"
ORACLE="$(state_get oracle)"; [[ -n "$ORACLE" ]] || die "run setup.sh first"
NAV="${1:-$(state_get nav)}"
TS="$(aligned_ms)"
log "NAV $NAV at slot $TS"
inv "$ORACLE" "$ADMIN_ID" -- set_price --updates "[\"$NAV\"]" --timestamp "$TS" >/dev/null
state_set nav "$NAV"
