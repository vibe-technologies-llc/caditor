#!/bin/sh
set -eu

MAX_GLIBC=${CADITOR_MAX_GLIBC:-2.35}
ALLOWED_LIBRARIES="libc.so.6 libm.so.6 libdl.so.2 libpthread.so.0 librt.so.1 libgcc_s.so.1 ld-linux-x86-64.so.2"

usage() {
    cat <<EOF
Usage: packaging/check-binary.sh PROGRAM

Checks what docs/RELEASING.md says of the Linux program: that it links only glibc and
libgcc_s (the libraries it names as needed are all in the list below, and every one
resolves with ldd), and that the highest glibc symbol version it uses is at most
$MAX_GLIBC, the glibc of Ubuntu 22.04 that packaging/INSTALL.md promises. CADITOR_MAX_GLIBC
names another limit, for previewing an archive built on a newer system.

Allowed libraries: $ALLOWED_LIBRARIES
EOF
}

fail() {
    echo "check-binary: $1" >&2
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

program=$1
[ -f "$program" ] || fail "$program does not exist"
for tool in objdump ldd sort; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is needed"
done

needed=$(objdump -p "$program" | sed -n 's/^ *NEEDED  *//p')
[ -n "$needed" ] || fail "$program names no needed library"
for library in $needed; do
    case " $ALLOWED_LIBRARIES " in
        *" $library "*) ;;
        *) fail "$program links $library, which is neither glibc nor libgcc_s" ;;
    esac
done

missing=$(ldd "$program" | grep "not found" || true)
[ -z "$missing" ] || fail "the dynamic loader cannot find: $missing"

highest=$(objdump -T "$program" | grep -o 'GLIBC_[0-9][0-9.]*' | sed 's/^GLIBC_//' | sort -Vu | tail -n 1)
[ -n "$highest" ] || fail "$program names no glibc version"
newest=$(printf '%s\n%s\n' "$highest" "$MAX_GLIBC" | sort -V | tail -n 1)
[ "$newest" = "$MAX_GLIBC" ] || fail "$program needs glibc $highest, newer than the $MAX_GLIBC it promises"

echo "check-binary: $(basename "$program") links $(echo "$needed" | tr '\n' ' ')and needs glibc $highest at most"
