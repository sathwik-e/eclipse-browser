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
clang -fobjc-arc -Wall -Wno-deprecated-declarations "$here/harness.m" "$work/menu.o" \
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

# The harness keeps WebKit's caches (menu.m's content rule list) out of the
# real home folder.
mkdir -p "$work/home"
run() {
    local env=(HARNESS_PORT="$port" HOME="$work/home" CFFIXED_USER_HOME="$work/home")
    if [[ "${GUARD_MALLOC:-}" == 1 ]]; then env+=(DYLD_INSERT_LIBRARIES=/usr/lib/libgmalloc.dylib); fi
    env "${env[@]}" "$work/harness" "$1" 2>/dev/null || true
}

scenarios=("$@")
if [[ ${#scenarios[@]} -eq 0 ]]; then
    scenarios=(legit stopwrite nocontent blankthen eclipse sameorigin nogesture five nested
               openernav sheet print2 printclose refocus focus resize menu linktab)
fi

for scenario in "${scenarios[@]}"; do
    echo "$scenario"
    out="$(run "$scenario")"
    if grep -q "NOT KEY" <<<"$out"; then
        # Synthetic clicks only reach WebKit in the active app's key window.
        echo "  FAIL: the harness could not become the active app (is the screen locked?)"
        failures=$((failures + 1))
        continue
    fi
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
        check "$out" "at most three windows per tab, the rest as tabs" "RESULT .* little=3 fallbacks=2"
        check "$out" "the tab knows it has popups open (no hibernation)" "RESULT .* count=3" ;;
    nested)
        # The popup's automatic window.open becomes a tab; a click inside
        # opens nested windows up to the per-tab limit.
        check "$out" "nested popups count toward the tab's limit" "RESULT .* little=3 " ;;
    openernav)
        # A popup from the other site sets opener.location to eclipse://
        # pages; the tab's own page and the browser still may.
        check "$out" "the popup tried to navigate its opener" "MSG popup:nav2 little=1"
        check "$out" "a popup's navigation of the tab is marked" \
            "POLICY eclipse://localhost/newtab ctx=5 allow=0"
        check "$out" "also once the popup tries Settings" "POLICY eclipse://localhost/settings ctx=5 allow=0"
        check "$out" "the tab's own page is not marked" "POLICY eclipse://localhost/newtab ctx=1 allow=1"
        check "$out" "the browser's load is not marked" "POLICY eclipse://localhost/notes ctx=1 allow=1"
        check "$out" "the tab ends where the browser sent it" "TABURL eclipse://localhost/notes" ;;
    sheet)
        # A confirm() sheet on the popup is key: shortcuts are the popup's.
        check "$out" "the dialog sheet is key" "KEY sheet"
        check "$out" "⌘R waits while the dialog is up" "PRESS cmd-r handled=1" "ACTION reload"
        check "$out" "⌘W closes the popup, not the tab" "RESULT .* little=0 " "ACTION close_tab" ;;
    print2)
        check "$out" "the popup printed twice" "MSG popup:printed little=1"
        check "$out" "one ⌘W closes the popup" "RESULT .* little=0 " "ACTION close_tab" ;;
    printclose)
        check "$out" "the popup closed itself" "RESULT .* little=0 "
        check "$out" "no closed popup is left behind" "RESULT" "LEFT " ;;
    refocus)
        check "$out" "the tab was key before the second click" "KEY tab"
        check "$out" "reopening the popup by name brings it to the front" "KEY little"
        check "$out" "the popup was reused" "RESULT .* little=1 "
        check "$out" "and navigated" "WINDOW .* url=http://$victim/pop.html\?n=2" ;;
    focus)
        check "$out" "the opener called popup.focus()" "MSG opener:focused"
        check "$out" "popup.focus() brings it to the front" "KEY little" ;;
    resize)
        check "$out" "the opener reads the popup's real size" "MSG opener:outer=[1-9][0-9]*x[1-9][0-9]*" \
            "MSG opener:outer=0x0"
        check "$out" "resizeTo/moveTo apply" "MSG popup:after outer=800x700 screen=200,120"
        check "$out" "the window has that frame" "WINDOW .* frame=200,120,800x700" ;;
    menu)
        check "$out" "the popup is key" "KEY little"
        check "$out" "⌘= zooms the popup" "ZOOM 1.10" "ACTION zoom_in"
        check "$out" "⌘D does not bookmark the tab behind" "PRESS cmd-d handled=1" "ACTION favorite_tab"
        check "$out" "⌘T opens a tab in the browser window" "ACTION new_tab"
        check "$out" "and brings it to the front" "KEY tab" ;;
    linktab)
        check "$out" "a script's tab stays behind the popup" "ACTION little_window_new_tab:http://$opener/pop.html\?auto"
        check "$out" "the popup stayed key" "KEY little"
        check "$out" "a clicked link opens a tab" "ACTION little_window_new_tab:http://$victim/pop.html\?terms"
        check "$out" "and brings the browser window forward" "KEY tab" ;;
    *)
        echo "  unknown scenario"; failures=$((failures + 1)) ;;
    esac
done

if [[ $failures -gt 0 ]]; then
    echo "$failures check(s) failed"
    exit 1
fi
echo "all checks passed"
