#!/bin/sh
# Example adapter for a custom child inside an external display wrapper.
# It records the child's wait status even if Gamescope later exits zero.
set -u
: "${GODOT_QA_RUN_ROOT:?set from artifact begin-run --out}"
: "${GODOT_QA_RUN_ID:?set from data.run.run_id}"
: "${GODOT_QA_RUN_NONCE:?set from data.run.nonce}"
: "${GODOT_QA_RUN_STARTED_AT_UNIX_MS:?set from data.run.started_at_unix_ms}"
case "$GODOT_QA_RUN_ID" in run-*) ;; *) exit 2 ;; esac
run_hex=${GODOT_QA_RUN_ID#run-}
case "$run_hex" in *[!0123456789abcdef]*|'') exit 2 ;; esac
case "$GODOT_QA_RUN_NONCE" in *[!0123456789abcdef]*|'') exit 2 ;; esac
case "$GODOT_QA_RUN_STARTED_AT_UNIX_MS" in *[!0123456789]*|'') exit 2 ;; esac
if [ "${#run_hex}" -ne 32 ] || [ "${#GODOT_QA_RUN_NONCE}" -ne 32 ]; then
  exit 2
fi

"$@"
child_code=$?
completed_at=$(date +%s%3N)
termination=exited
# Shell wait status cannot distinguish exit 128+N from signal N.
if [ "$child_code" -ge 128 ]; then
  termination=unknown
fi
receipt_tmp="$GODOT_QA_RUN_ROOT/child-status.json.tmp.$$"
printf '{"schema_version":1,"run_id":"%s","nonce":"%s","started_at_unix_ms":%s,"completed_at_unix_ms":%s,"termination":"%s","exit_code":%s}\n' \
  "$GODOT_QA_RUN_ID" "$GODOT_QA_RUN_NONCE" "$GODOT_QA_RUN_STARTED_AT_UNIX_MS" "$completed_at" "$termination" "$child_code" > "$receipt_tmp"
mv "$receipt_tmp" "$GODOT_QA_RUN_ROOT/child-status.json"
exit "$child_code"
