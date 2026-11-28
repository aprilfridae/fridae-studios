#!/bin/sh
# usage: run.sh [ui folder]  - the toolbox on the page: start mock.py on the page's ui folder, run page.js
D=$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || pwd); ui=${1:-${NEW_UI:-H:/fridae-app-t4/ui}}; port=${PORTA:-8799}
OUTD=${OUTD:-H:/rust/tmp/rvwork-apptools}
kill_it() { pid=$(netstat -ano | grep "127.0.0.1:$port .*LISTENING" | awk '{print $5}' | head -1); [ -n "$pid" ] && taskkill //F //PID $pid >/dev/null 2>&1; }
kill_it
mkdir -p "$OUTD/shots"
MOCK_UI=$ui H:/fridae-venv/Scripts/python "$D/mock.py" $port >"$OUTD/mock.log" 2>&1 &
sleep 1
o=$OUTD/out.txt
OUT=$OUTD/shots/ PORT=$port PLAYWRIGHT_BROWSERS_PATH=H:/ms-playwright H:/node/node.exe "$D/page.js" > "$o" 2>&1
echo "PASS: $(grep -c '"PASS"' "$o")  FAIL: $(grep -c '"FAIL' "$o")"; grep '"FAIL\|CRASH' "$o" | cut -c1-600; grep -A8 '"errors"' "$o" | cut -c1-400
kill_it
