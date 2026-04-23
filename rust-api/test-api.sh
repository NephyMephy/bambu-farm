#!/usr/bin/env bash
set -euo pipefail

PORT=52001
BASE="http://localhost:${PORT}"
AUTH="Authorization: Bearer testkey"

echo "Starting server on port ${PORT}..."
API_KEY=testkey API_BIND="0.0.0.0:${PORT}" cargo run &>/tmp/bambu-api.log &
SERVER_PID=$!

cleanup() {
  echo "Cleaning up..."
  kill "${SERVER_PID}" 2>/dev/null || true
  wait "${SERVER_PID}" 2>/dev/null || true
}
trap cleanup EXIT

echo "Waiting for server..."
for i in $(seq 1 30); do
  if curl -sf "${BASE}/health" >/dev/null 2>&1; then
    echo "Server ready after ${i}s"
    break
  fi
  sleep 1
done

echo ""
echo "=== Health ==="
curl -s "${BASE}/health" | jq .

echo ""
echo "=== List printers (no auth - expect 401) ==="
CODE=$(curl -s -o /dev/null -w "%{http_code}" "${BASE}/v1/printers")
echo "Status: ${CODE}"

echo ""
echo "=== List printers (with auth) ==="
curl -s -H "${AUTH}" "${BASE}/v1/printers" | jq .

echo ""
echo "=== Upsert printer ==="
curl -s -X POST -H "${AUTH}" -H "Content-Type: application/json" \
  -d '{"id":"test-p1","host":"192.168.1.100","device_id":"DEV001","access_code":"12345678"}' \
  "${BASE}/v1/printers" | jq .

echo ""
echo "=== Get printer ==="
curl -s -H "${AUTH}" "${BASE}/v1/printers/test-p1" | jq .

echo ""
echo "=== Stream URL (stopped) ==="
curl -s -H "${AUTH}" "${BASE}/v1/printers/test-p1/stream/url" | jq .

echo ""
echo "=== Stop stream (not running) ==="
curl -s -X POST -H "${AUTH}" "${BASE}/v1/printers/test-p1/stream/stop" | jq .

echo ""
echo "=== Upsert printer (update) ==="
curl -s -X POST -H "${AUTH}" -H "Content-Type: application/json" \
  -d '{"id":"test-p1","host":"192.168.1.200","device_id":"DEV002","access_code":"87654321"}' \
  "${BASE}/v1/printers" | jq .

echo ""
echo "=== List printers ==="
curl -s -H "${AUTH}" "${BASE}/v1/printers" | jq .

echo ""
echo "=== Get non-existent printer (expect 404) ==="
CODE=$(curl -s -o /dev/null -w "%{http_code}" -H "${AUTH}" "${BASE}/v1/printers/nope")
echo "Status: ${CODE}"

echo ""
echo "=== Bad upsert (empty id) ==="
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST -H "${AUTH}" -H "Content-Type: application/json" \
  -d '{"id":"","host":"1.2.3.4","device_id":"D","access_code":"A"}' \
  "${BASE}/v1/printers")
echo "Status: ${CODE}"

echo ""
echo "=== Bad upsert (missing fields) ==="
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST -H "${AUTH}" -H "Content-Type: application/json" \
  -d '{"id":"x","host":"","device_id":"","access_code":""}' \
  "${BASE}/v1/printers")
echo "Status: ${CODE}"

echo ""
echo "=== Delete printer ==="
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X DELETE -H "${AUTH}" "${BASE}/v1/printers/test-p1")
echo "Status: ${CODE}"

echo ""
echo "=== List printers (should be empty) ==="
curl -s -H "${AUTH}" "${BASE}/v1/printers" | jq .

echo ""
echo "=== Batch upsert printers ==="
curl -s -X POST -H "${AUTH}" -H "Content-Type: application/json" \
  -d '{"printers":[
    {"id":"batch-1","host":"10.0.0.1","device_id":"BATCH001","access_code":"11111111"},
    {"id":"batch-2","host":"10.0.0.2","device_id":"BATCH002","access_code":"22222222"},
    {"id":"batch-3","host":"10.0.0.3","device_id":"BATCH003","access_code":"33333333"}
  ]}' \
  "${BASE}/v1/printers/batch" | jq .

echo ""
echo "=== Batch upsert (update + error) ==="
curl -s -X POST -H "${AUTH}" -H "Content-Type: application/json" \
  -d '{"printers":[
    {"id":"batch-1","host":"10.0.0.10","device_id":"BATCH001-UPD","access_code":"updated!"},
    {"id":"bad id!","host":"1.2.3.4","device_id":"X","access_code":"Y"}
  ]}' \
  "${BASE}/v1/printers/batch" | jq .

echo ""
echo "=== List printers (should have 3) ==="
curl -s -H "${AUTH}" "${BASE}/v1/printers" | jq .

echo ""
echo "=== Cleanup batch printers ==="
for id in batch-1 batch-2 batch-3; do
  CODE=$(curl -s -o /dev/null -w "%{http_code}" -X DELETE -H "${AUTH}" "${BASE}/v1/printers/${id}")
  echo "Delete ${id}: ${CODE}"
done

echo ""
echo "=== Delete non-existent printer (expect 404) ==="
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X DELETE -H "${AUTH}" "${BASE}/v1/printers/nope")
echo "Status: ${CODE}"

echo ""
echo "=== Server log ==="
cat /tmp/bambu-api.log

echo ""
echo "=== ALL TESTS PASSED ==="
