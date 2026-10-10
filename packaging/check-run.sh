#!/bin/sh
set -eu

START_SECONDS=${CADITOR_CHECK_SECONDS:-90}
FIRST_FRAME="the first frame was drawn"
EDIT_JOURNALLED="startup check: an edit is in the recovery journal"
RECOVERY_OFFERED="recovery offered for .* with [1-9][0-9]* recoverable changes"
MODEL_SAVED="startup check: saved the model to"
MODEL_NAME=model.caditor

usage() {
    cat <<EOF
Usage: packaging/check-run.sh PROGRAM

Starts PROGRAM under Xvfb on Mesa's software drivers, first on Vulkan (lavapipe) and then on
OpenGL (llvmpipe, WGPU_BACKEND=gl), or only on the backend WGPU_BACKEND names. On each it runs
two checks, each with a state directory of its own. In both, the program reaches its first
frame and, with CADITOR_STARTUP_CHECK set, journals an edit; it is killed with SIGKILL, its
recovery journal is checked to be left behind, and it is started again with
CADITOR_STARTUP_CHECK=restore, which makes it accept the recovery offer, and must log that the
edit came back. The first check does this with an untitled document; the second first has the
program save a small model, then opens that model for the edit and the restore. Needs Xvfb.
EOF
}

fail() {
    echo "check-run: $1" >&2
    for output in "$work"/run-"$label"*.log "$work"/run-xvfb.log; do
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
label=""
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
XDG_CONFIG_HOME="$work/config"
XDG_DATA_HOME="$work/data"
XDG_RUNTIME_DIR="$work/runtime"
mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
export HOME XDG_CONFIG_HOME XDG_DATA_HOME XDG_RUNTIME_DIR

if [ -z "${VK_ICD_FILENAMES:-}" ]; then
    for icd in /usr/share/vulkan/icd.d/lvp_icd*.json /etc/vulkan/icd.d/lvp_icd*.json; do
        if [ -f "$icd" ]; then
            VK_ICD_FILENAMES=$icd
            export VK_ICD_FILENAMES
            break
        fi
    done
fi
LIBGL_ALWAYS_SOFTWARE=1
export LIBGL_ALWAYS_SOFTWARE

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

start() {
    log=$1
    check=$2
    shift 2
    CADITOR_STARTUP_CHECK=$check "$program" "$@" >"$log" 2>&1 &
    app=$!
}

stop() {
    kill -"$1" "$app"
    wait "$app" 2>/dev/null || true
    app=""
}

fresh_state() {
    XDG_STATE_HOME="$work/state-$label"
    rm -rf "$XDG_STATE_HOME"
    mkdir -p "$XDG_STATE_HOME"
    export XDG_STATE_HOME
    journals="$XDG_STATE_HOME/caditor/recovery"
}

journal_of_the_model() {
    printf '%s/.%s.journal' "$work/models" "$MODEL_NAME"
}

edit_killed_then_restored() {
    step=$1
    expected=$2
    shift 2
    edited="$work/run-$label-$step-edit.log"
    restored="$work/run-$label-$step-restore.log"

    start "$edited" 1 "$@"
    wait_for "$edited" "$FIRST_FRAME"
    wait_for "$edited" "$EDIT_JOURNALLED"
    stop KILL

    found=$(find "$journals" "$work/models" -name '*.journal' 2>/dev/null | head -n 1)
    [ -n "$found" ] || fail "the killed program left no recovery journal"
    [ "$step" != model ] || [ "$found" = "$(journal_of_the_model)" ] ||
        fail "the journal of the opened model is not beside it: $found"
    ! grep -q "stopping on signal" "$edited" || fail "the first run did not end by SIGKILL"

    start "$restored" restore "$@"
    wait_for "$restored" "$FIRST_FRAME"
    wait_for "$restored" "$RECOVERY_OFFERED"
    wait_for "$restored" "startup check: restored [1-9][0-9]* changes of $expected\$"
    stop TERM
}

check_untitled() {
    label="$1-untitled"
    fresh_state
    edit_killed_then_restored untitled Untitled
}

check_model() {
    label="$1-model"
    fresh_state
    model="$work/models/$MODEL_NAME"
    mkdir -p "$work/models"
    saved="$work/run-$label-save.log"

    start "$saved" "save:$model"
    wait_for "$saved" "$FIRST_FRAME"
    wait_for "$saved" "$MODEL_SAVED"
    stop TERM
    [ -s "$model" ] || fail "the program did not leave the model $model"

    fresh_state
    edit_killed_then_restored model "$MODEL_NAME" "$model"
}

if [ -n "${WGPU_BACKEND:-}" ]; then
    backends=$WGPU_BACKEND
else
    backends="vulkan gl"
fi
for backend in $backends; do
    WGPU_BACKEND=$backend
    export WGPU_BACKEND
    check_untitled "$backend"
    check_model "$backend"
done

echo "Started $(basename "$program") on $backends to a first frame, killed it with an edit journalled and restored the edit, for an untitled document and for an opened model."
