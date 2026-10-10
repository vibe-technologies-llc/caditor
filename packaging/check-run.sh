#!/bin/sh
set -eu

START_SECONDS=${CADITOR_CHECK_SECONDS:-90}
FIRST_FRAME="the first frame was drawn"
EDIT_JOURNALLED="startup check: an edit is in the recovery journal"
RECOVERY_OFFERED="recovery offered for .* with [1-9][0-9]* recoverable changes"

usage() {
    cat <<EOF
Usage: packaging/check-run.sh PROGRAM

Starts PROGRAM under Xvfb on a software Vulkan driver (Mesa lavapipe, or the backend named
by WGPU_BACKEND) with a state directory of its own, and checks that it reaches its first
frame and, with CADITOR_STARTUP_CHECK set, journals an edit. It then kills the program with
SIGKILL, checks that its recovery journal is left behind, starts it again and checks that
it offers to restore the edit. Needs Xvfb.
EOF
}

fail() {
    echo "check-run: $1" >&2
    for output in "$work"/run-*.log; do
        if [ -f "$output" ]; then
            echo "--- $(basename "$output")" >&2
            tail -n 40 "$output" >&2
        fi
    done
    exit 1
}

if [ $# -ne 1 ]; then
    usage >&2
    exit 2
fi
case "$1" in
    -h | --help)
        usage
        exit 0
        ;;
esac

program=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
work=$(mktemp -d)
server=""
app=""
cleanup() {
    for pid in $app $server; do
        kill -KILL "$pid" 2>/dev/null || true
    done
    rm -rf "$work"
}
trap cleanup EXIT INT TERM

[ -x "$program" ] || fail "$program is not an executable file"
command -v Xvfb >/dev/null 2>&1 || fail "Xvfb is needed"

Xvfb -displayfd 3 -screen 0 1280x800x24 -nolisten tcp 3>"$work/display" 2>"$work/run-xvfb.log" &
server=$!
waited=0
while [ ! -s "$work/display" ]; do
    kill -0 "$server" 2>/dev/null || fail "Xvfb stopped before it was ready"
    [ "$waited" -lt 30 ] || fail "Xvfb was not ready after 30 seconds"
    sleep 1
    waited=$((waited + 1))
done
DISPLAY=":$(tr -d '\n' <"$work/display")"
export DISPLAY
unset WAYLAND_DISPLAY

HOME="$work/home"
XDG_STATE_HOME="$work/state"
XDG_CONFIG_HOME="$work/config"
XDG_DATA_HOME="$work/data"
XDG_RUNTIME_DIR="$work/runtime"
mkdir -p "$HOME" "$XDG_STATE_HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
export HOME XDG_STATE_HOME XDG_CONFIG_HOME XDG_DATA_HOME XDG_RUNTIME_DIR

if [ -z "${VK_ICD_FILENAMES:-}" ]; then
    for icd in /usr/share/vulkan/icd.d/lvp_icd*.json /etc/vulkan/icd.d/lvp_icd*.json; do
        if [ -f "$icd" ]; then
            VK_ICD_FILENAMES=$icd
            export VK_ICD_FILENAMES
            break
        fi
    done
fi
WGPU_BACKEND=${WGPU_BACKEND:-vulkan}
export WGPU_BACKEND
LIBGL_ALWAYS_SOFTWARE=1
export LIBGL_ALWAYS_SOFTWARE

journals="$XDG_STATE_HOME/caditor/recovery"

wait_for() {
    log=$1
    pattern=$2
    waited=0
    while ! grep -Eq "$pattern" "$log" 2>/dev/null; do
        kill -0 "$app" 2>/dev/null || fail "the program stopped before it logged: $pattern"
        [ "$waited" -lt "$START_SECONDS" ] || fail "no '$pattern' after $START_SECONDS seconds"
        sleep 1
        waited=$((waited + 1))
    done
}

CADITOR_STARTUP_CHECK=1 "$program" >"$work/run-1.log" 2>&1 &
app=$!
wait_for "$work/run-1.log" "$FIRST_FRAME"
wait_for "$work/run-1.log" "$EDIT_JOURNALLED"

kill -KILL "$app"
wait "$app" 2>/dev/null || true
app=""

found=$(find "$journals" -name '*.journal' 2>/dev/null | head -n 1)
[ -n "$found" ] || fail "the killed program left no recovery journal under $journals"
! grep -q "stopping on signal" "$work/run-1.log" || fail "the first run did not end by SIGKILL"

"$program" >"$work/run-2.log" 2>&1 &
app=$!
wait_for "$work/run-2.log" "$FIRST_FRAME"
wait_for "$work/run-2.log" "$RECOVERY_OFFERED"

kill -TERM "$app"
wait "$app" 2>/dev/null || true
app=""

echo "Started $(basename "$program") to a first frame, killed it with an edit journalled and recovered the edit."
