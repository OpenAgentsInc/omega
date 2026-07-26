#!/usr/bin/env sh
set -eu

# Uninstalls Omega. OMEGA-DELTA-0036.
#
# This script knows no paths of its own. Every path it removes is handed to it
# by the caller in OMEGA_UNINSTALL_PATHS, one absolute path per line, and that
# list is derived on the Rust side from the same `paths::` functions the running
# application uses to write those directories in the first place. See
# `crates/cli/src/uninstall.rs`.
#
# It is written this way because of what it used to be. Up to 0.2.0-rc14 this
# file was upstream's uninstaller, unchanged: it deleted the upstream editor's
# application bundle, application-support tree, logs, preferences and remote
# server directory, announced that the upstream editor had been uninstalled, and
# removed no Omega path at all. A command advertised as "Uninstall Omega" kept
# Omega and destroyed the user's other editor. A hard-coded path table is how
# that happened, and a second hard-coded path table is not the fix, so there is
# no path table here.
#
# Contract:
#
#   OMEGA_UNINSTALL_PRODUCT     display name, e.g. "Omega RC"          required
#   OMEGA_UNINSTALL_PATHS       newline-separated absolute paths       required
#   OMEGA_UNINSTALL_CONFIG_DIR  removed only if the user says so       optional
#   OMEGA_UNINSTALL_DRY_RUN     when "1", print the plan, remove nothing
#   OMEGA_UNINSTALL_ASSUME_YES  when "1", remove the config directory unasked
#
# Refusing is the safe direction: an unset or empty contract exits non-zero
# rather than falling back to a default, because every default this file has
# ever had belonged to somebody else's product.

refuse() {
    echo "omega --uninstall: $1" >&2
    exit 1
}

product="${OMEGA_UNINSTALL_PRODUCT:-}"
paths="${OMEGA_UNINSTALL_PATHS:-}"
config_dir="${OMEGA_UNINSTALL_CONFIG_DIR:-}"
dry_run="${OMEGA_UNINSTALL_DRY_RUN:-}"
assume_yes="${OMEGA_UNINSTALL_ASSUME_YES:-}"

[ -n "$product" ] || refuse "OMEGA_UNINSTALL_PRODUCT is not set; refusing to guess what to remove"
[ -n "$paths" ] || refuse "OMEGA_UNINSTALL_PATHS is empty; refusing to guess what to remove"

check_path() {
    case "$1" in
        /*) ;;
        *) refuse "refusing to remove the relative path '$1'" ;;
    esac
    case "$1" in
        /|/Applications|/Users|/System|/Library|/usr|/bin|/etc|/var|/tmp|"$HOME")
            refuse "refusing to remove '$1'"
            ;;
    esac
}

# Everything is checked before anything is removed. A here-document keeps the
# loop in this shell, so a refusal here really does stop the run.
while IFS= read -r path; do
    [ -n "$path" ] || continue
    check_path "$path"
done <<PLAN
$paths
PLAN

if [ -n "$config_dir" ]; then
    check_path "$config_dir"
fi

if [ "$dry_run" = "1" ]; then
    echo "plan: $product"
    while IFS= read -r path; do
        [ -n "$path" ] || continue
        echo "remove: $path"
    done <<PLAN
$paths
PLAN
    if [ -n "$config_dir" ]; then
        echo "prompt: $config_dir"
    fi
    echo "$product uninstall plan printed; nothing was removed"
    exit 0
fi

while IFS= read -r path; do
    [ -n "$path" ] || continue
    if [ -e "$path" ] || [ -L "$path" ]; then
        rm -rf "$path"
        echo "removed $path"
    else
        echo "absent $path"
    fi
done <<PLAN
$paths
PLAN

if [ -n "$config_dir" ] && { [ -e "$config_dir" ] || [ -L "$config_dir" ]; }; then
    if [ "$assume_yes" = "1" ]; then
        rm -rf "$config_dir"
        echo "removed $config_dir"
    elif [ -t 0 ]; then
        printf 'Do you want to keep your %s settings and keymap (%s)? [Y/n] ' "$product" "$config_dir"
        response=""
        read -r response || response=""
        case "$response" in
            [nN]|[nN][oO])
                rm -rf "$config_dir"
                echo "removed $config_dir"
                ;;
            *)
                echo "kept $config_dir"
                ;;
        esac
    else
        echo "kept $config_dir"
    fi
fi

echo "$product has been uninstalled"
