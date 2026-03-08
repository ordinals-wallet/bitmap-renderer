#!/bin/bash
# Usage: ./tests/add_test.sh <block_number> <inscription_id>
#
# Example:
#   ./tests/add_test.sh 163284 ce889af2c08782abfc7e142d7f4c214bcd53795f9338a232ccb4dff48657053bi0
#
# This downloads the ME reference image and saves the block data fixture.
# Requires: BTC_RPC_USER, BTC_RPC_PASS env vars (or defaults to bitcoin/bitcoin)

set -e

BLOCK=$1
INSCRIPTION=$2
DIR="$(cd "$(dirname "$0")" && pwd)"
RPC_USER="${BTC_RPC_USER:-bitcoin}"
RPC_PASS="${BTC_RPC_PASS:-bitcoin}"
RPC_URL="${BTC_RPC_URL:-http://localhost:8332}"

if [ -z "$BLOCK" ] || [ -z "$INSCRIPTION" ]; then
    echo "Usage: $0 <block_number> <inscription_id>"
    echo "  block_number:   Bitcoin block height"
    echo "  inscription_id: Ordinals inscription ID for the bitmap"
    exit 1
fi

echo "Adding test case for block $BLOCK..."

# Download ME reference image
echo "  Downloading reference image..."
curl -sS -o "$DIR/references/$BLOCK.png" \
    "https://bitmap-img.magiceden.dev/v1/$INSCRIPTION"
echo "  Saved: tests/references/$BLOCK.png"

# Save block data fixture
echo "  Fetching block data from RPC..."
HASH=$(curl -s -u "$RPC_USER:$RPC_PASS" "$RPC_URL" \
    -d "{\"jsonrpc\":\"1.0\",\"id\":\"1\",\"method\":\"getblockhash\",\"params\":[$BLOCK]}" \
    | python3 -c "import sys,json; print(json.load(sys.stdin)['result'])")

curl -s -u "$RPC_USER:$RPC_PASS" "$RPC_URL" \
    -d "{\"jsonrpc\":\"1.0\",\"id\":\"1\",\"method\":\"getblock\",\"params\":[\"$HASH\",2]}" \
    | python3 -c "
import sys, json
data = json.load(sys.stdin)['result']
slim = {'tx': [{'vout': [{'value': o['value']} for o in tx['vout']]} for tx in data['tx']]}
json.dump(slim, sys.stdout)
" > "$DIR/fixtures/$BLOCK.json"
echo "  Saved: tests/fixtures/$BLOCK.json"

echo ""
echo "Done! Now add this to tests/regression.rs:"
echo ""
echo "  #[test]"
echo "  fn block_${BLOCK}() { assert_block_matches(${BLOCK}, 0.70, 0.50); }"
