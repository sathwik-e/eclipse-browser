#!/usr/bin/env bash
# End-to-end checks for Little Eclipse windows (src/menu.m): builds menu.m
# with harness.m, serves www/ on two local ports and runs each scenario.
# Needs a logged-in macOS session (it opens windows for a few seconds each).
#
#   scripts/little-eclipse-e2e/run.sh [scenario ...]
#
# HARNESS_PORT (default 8831) and HARNESS_PORT+1 must be free. Set
# GUARD_MALLOC=1 to run the harness under libgmalloc.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
port="${HARNESS_PORT:-8831}"
work="$(mktemp -d "${TMPDIR:-/tmp}/little-e2e.XXXXXX")"
pids=()
cleanup() {
    for pid in "${pids[@]+"${pids[@]}"}"; do
        kill "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
    done
    rm -rf "$work"
}
trap cleanup EXIT

clang -fobjc-arc -Wall -Wno-deprecated-declarations -c "$repo/src/menu.m" -o "$work/menu.o"
clang -fobjc-arc -Wall "$here/harness.m" "$work/menu.o" \
    -framework Cocoa -framework WebKit -framework QuartzCore -o "$work/harness"

for p in "$port" "$((port + 1))"; do
    python3 -I "$here/server.py" "$here/www" "$p" 2>"$work/server-$p.log" &
    pids+=("$!")
done
for _ in $(seq 50); do
    curl -fs -o /dev/null "http://127.0.0.1:$port/pop.html" &&
        curl -fs -o /dev/null "http://127.0.0.1:$((port + 1))/pop.html" && break
    sleep 0.1
done

victim="localhost:$((port + 1))"
opener="127.0.0.1:$port"
failures=0

# check <output> <description> <extended regex that must match> [regex that must not match]
check() {
    local output="$1" description="$2" must="$3" mustnot="${4:-}"
    if ! grep -Eq -- "$must" <<<"$output"; then
        echo "  FAIL: $description (no match for: $must)"
        failures=$((failures + 1))
        return
    fi
    if [[ -n "$mustnot" ]] && grep -Eq -- "$mustnot" <<<"$output"; then
        echo "  FAIL: $description (unexpected match for: $mustnot)"
        failures=$((failures + 1))
        return
    fi
    echo "  ok: $description"
}

run() {
    local env=(HARNESS_PORT="$port")
    if [[ "${GUARD_MALLOC:-}" == 1 ]]; then env+=(DYLD_INSERT_LIBRARIES=/usr/lib/libgmalloc.dylib); fi
    env "${env[@]}" "$work/harness" "$1" 2>/dev/null
}

scenarios=("$@")
if [[ ${#scenarios[@]} -eq 0 ]]; then
    scenarios=(legit stopwrite nocontent blankthen eclipse sameorigin nogesture five nested)
fi

for scenario in "${scenarios[@]}"; do
    echo "$scenario"
    out="$(run "$scenario")"
    case "$scenario" in
    legit)
        check "$out" "a committed page shows its host" "subtitle=\"Not Secure — $victim\"" ;;
    stopwrite | nocontent | blankthen)
        # The opener wrote a fake page into the popup's initial document
        # after its load to the other site was stopped / got a 204.
        check "$out" "the opener wrote into the popup" "MSG opener:wrote:Sign in"
        check "$out" "the subtitle shows the document's real host" \
            "WINDOW title=\"Sign in\" subtitle=\"Not Secure — $opener\"" "$victim" ;;
    eclipse)
        check "$out" "eclipse:// subresources fail in the popup" "MSG popup:img=error&fetch=err little=1"
        check "$out" "the scheme handler never sees the popup's view" "unsafe_scheme=0" "UNSAFE" ;;
    sameorigin)
        check "$out" "the tab's own messages are the tab's" "MSG tab:self little=0"
        check "$out" "a same-origin popup's messages are marked" \
            "MSG popup:url=http://$opener/popup-callback\\?state=done little=1" ;;
    nogesture)
        check "$out" "a popup without a gesture opens as a tab" "RESULT .* little=0 fallbacks=1" ;;
    five)
        check "$out" "at most three windows per tab, the rest as tabs" "RESULT .* little=3 fallbacks=2" ;;
    nested)
        # The popup's automatic window.open becomes a tab; a click inside
        # opens nested windows up to the per-tab limit.
        check "$out" "nested popups count toward the tab's limit" "RESULT .* little=3 " ;;
    *)
        echo "  unknown scenario"; failures=$((failures + 1)) ;;
    esac
done

if [[ $failures -gt 0 ]]; then
    echo "$failures check(s) failed"
    exit 1
fi
echo "all checks passed"
