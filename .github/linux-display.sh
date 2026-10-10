#!/bin/sh
set -eu

action=${1:-}
set -- xvfb libvulkan1 mesa-vulkan-drivers libegl-mesa0 libegl1 libgl1 libgl1-mesa-dri libxcursor1 libxi6 \
    libxkbcommon-x11-0 libxrandr2
export DEBIAN_FRONTEND=noninteractive

case "$action" in
    install)
        apt-get update
        apt-get install -y --no-install-recommends "$@"
        ;;
    remove)
        apt-get purge -y --auto-remove "$@"
        ;;
    *)
        echo "Usage: .github/linux-display.sh install|remove" >&2
        exit 2
        ;;
esac
