#!/bin/sh
# Seeds the disposable Muninn database with the synthetic records of a run.
#
# The script runs inside the internal container network: it starts the product
# bridge on the loopback interface, writes the records through it, and exits.
# No port is published and no host path is written.
set -eu

: "${BENCH_SEED_PLAN:?the seed plan path is required}"

PLAN="${BENCH_SEED_PLAN}"
BRIDGE_URL="http://127.0.0.1:3001"
IDS_FILE="/tmp/seed-ids.txt"

log() {
  echo "[seed] $*" >&2
}

fail() {
  log "ERROR: $*"
  exit 70
}

command -v jq >/dev/null 2>&1 || fail "jq is missing from the image"
[ -f "${PLAN}" ] || fail "seed plan not found: ${PLAN}"

log "creating the schema and indexes"
muninn schema >/tmp/schema.log 2>&1 || {
  cat /tmp/schema.log >&2 || true
  fail "schema setup failed"
}

log "starting the bridge on ${BRIDGE_URL}"
muninn bridge --host 127.0.0.1 --port 3001 >/tmp/bridge.log 2>&1 &
BRIDGE_PID=$!
trap 'kill "${BRIDGE_PID}" 2>/dev/null || true' EXIT

attempt=0
until curl -sS -o /dev/null -X POST \
    -H 'content-type: application/json' \
    -d '{"query":"readiness probe","limit":1}' \
    "${BRIDGE_URL}/search_messages" 2>/dev/null; do
  attempt=$((attempt + 1))
  if [ "${attempt}" -ge 60 ]; then
    cat /tmp/bridge.log >&2 || true
    fail "the bridge did not become ready"
  fi
  sleep 1
done
log "the bridge is ready"

: >"${IDS_FILE}"
seeded=0

jq -c '.records[]' "${PLAN}" | while IFS= read -r record; do
  index=$(printf '%s' "${record}" | jq -r '.index')
  kind=$(printf '%s' "${record}" | jq -r '.kind')
  owner=$(printf '%s' "${record}" | jq -r '.owner')
  text=$(printf '%s' "${record}" | jq -r '.text')

  if [ "${kind}" = "preference" ]; then
    category=$(printf '%s' "${record}" | jq -r '.category')
    body=$(jq -nc --arg c "${category}" --arg p "${text}" --arg u "${owner}" \
      '{category:$c, preference:$p, user_identifier:$u}')
    response=$(curl -sS -X POST -H 'content-type: application/json' \
      -d "${body}" "${BRIDGE_URL}/add_preference")
  else
    session=$(printf '%s' "${record}" | jq -r '.session_id')
    body=$(jq -nc --arg s "${session}" --arg c "${text}" --arg u "${owner}" \
      '{session_id:$s, role:"user", content:$c, user_identifier:$u}')
    response=$(curl -sS -X POST -H 'content-type: application/json' \
      -d "${body}" "${BRIDGE_URL}/add_message")
  fi

  identifier=$(printf '%s' "${response}" | jq -r '.id // empty')
  if [ -z "${identifier}" ]; then
    fail "the bridge did not return an identifier for record ${index}"
  fi
  printf '%s %s\n' "${index}" "${identifier}" >>"${IDS_FILE}"
done

seeded=$(wc -l <"${IDS_FILE}" | tr -d ' ')
log "seeded ${seeded} records"

jq -c '.supersedes[]?' "${PLAN}" | while IFS= read -r link; do
  old_index=$(printf '%s' "${link}" | jq -r '.old_index')
  new_index=$(printf '%s' "${link}" | jq -r '.new_index')
  old_id=$(awk -v key="${old_index} " '$1 == key {print $2}' "${IDS_FILE}")
  new_id=$(awk -v key="${new_index} " '$1 == key {print $2}' "${IDS_FILE}")
  [ -n "${old_id}" ] && [ -n "${new_id}" ] || fail "cannot resolve the superseded pair"
  body=$(jq -nc --arg o "${old_id}" --arg n "${new_id}" '{old_id:$o, new_id:$n}')
  curl -sS -o /dev/null -X POST -H 'content-type: application/json' \
    -d "${body}" "${BRIDGE_URL}/supersede_preference"
  log "superseded ${old_index} with ${new_index}"
done

log "seeding complete"
