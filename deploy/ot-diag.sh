#!/bin/sh
set -eu

STATE=${OT_DIAG_STATE:-${STATE_DIRECTORY:-/var/lib/ot-diag}}
SYSFS=${OT_DIAG_SYSFS:-/sys/class/power_supply}
TO=${OT_DIAG_TO:-}
TO_USER=${OT_DIAG_TO_USER:-}
HOURS=${OT_DIAG_WINDOW_HOURS:-12}
DEBOUNCE=${OT_DIAG_DEBOUNCE:-30}
MIN_INTERVAL=${OT_DIAG_MIN_INTERVAL:-600}

DISK_PATTERN='^[^ ]+ (nvme [^ ]+: .*(timeout|reset|controller is down|abort|failed|error)'
DISK_PATTERN="$DISK_PATTERN|ata[0-9.]+: .*(exception|failed command|error|hard resetting))"
DISK_PATTERN="$DISK_PATTERN|I/O error|medium error|unrecovered read error"
DISK_PATTERN="$DISK_PATTERN|EXT4-fs error|BTRFS (error|critical)|XFS .*(error|corruption)"
DISK_PATTERN="$DISK_PATTERN|remounting filesystem read-only"

attr() {
    if [ -r "$1/$2" ]; then cat "$1/$2"; fi
}

power_source() {
    found=0
    for d in "$SYSFS"/*; do
        case $(attr "$d" type) in Mains | USB) ;; *) continue ;; esac
        [ "$(attr "$d" scope)" != Device ] || continue
        [ -r "$d/online" ] || continue
        found=1
        if [ "$(attr "$d" online)" = 1 ]; then
            echo mains
            return
        fi
    done
    if [ "$found" = 1 ]; then
        echo battery
    elif [ "$(attr "$(battery_dir)" status)" = Discharging ]; then
        echo battery
    else
        echo mains
    fi
}

battery_dir() {
    for d in "$SYSFS"/*; do
        [ "$(attr "$d" type)" = Battery ] || continue
        [ "$(attr "$d" scope)" != Device ] || continue
        echo "$d"
        return
    done
}

power_line() {
    if [ "$(power_source)" = mains ]; then source=secteur; else source=batterie; fi
    bat=$(battery_dir)
    if [ -z "$bat" ]; then
        echo "Alimentation : $source, pas de batterie"
        return
    fi
    case $(attr "$bat" status) in
        Charging) state=" (en charge)" ;;
        Discharging) state=" (en décharge)" ;;
        Full) state=" (chargée)" ;;
        "Not charging") state="" ;;
        *) state=" (état inconnu)" ;;
    esac
    echo "Alimentation : $source, $(attr "$bat" capacity) %$state"
}

disk_lines() {
    if [ -z "$(journalctl -k -b -q --no-pager -n 1 -o cat)" ]; then
        echo "SSD : journal du noyau illisible, diagnostic impossible"
        return
    fi
    log=$(journalctl _TRANSPORT=kernel -q --no-pager -o short-iso -p warning --since "-${HOURS}h")
    alerts=$(printf '%s\n' "$log" \
        | sed -n 's/^[0-9-]*T\([0-9][0-9]:[0-9][0-9]\)[^ ]* [^ ]* kernel: \(.*\)$/\1 \2/p' \
        | grep -E -i -e "$DISK_PATTERN" || [ $? -eq 1 ])
    if [ -z "$alerts" ]; then
        echo "SSD : aucune alerte sur $HOURS h"
        return
    fi
    count=$(printf '%s\n' "$alerts" | wc -l | tr -d ' ')
    last=$(printf '%s\n' "$alerts" | tail -n 1 | cut -d ' ' -f 1)
    if [ "$count" = 1 ]; then
        echo "SSD : 1 erreur détectée sur $HOURS h, à $last :"
    else
        echo "SSD : $count erreurs détectées sur $HOURS h, dernière à $last :"
    fi
    printf '%s\n' "$alerts" | cut -d ' ' -f 2- | tac | awk '!seen[$0]++' | head -n 3 | tac \
        | cut -c 1-100 | sed 's/^/- /'
}

build_report() {
    power_line
    disk_lines
}

send() {
    message=$1
    if [ -z "$TO" ] && [ -z "$TO_USER" ]; then
        echo "aucune cible : OT_DIAG_TO (appareils de mon groupe) ou OT_DIAG_TO_USER (personnes) est obligatoire" >&2
        return 2
    fi
    if [ -n "$TO_USER" ]; then
        if [ -n "$TO" ]; then
            echo "OT_DIAG_TO (appareils de mon groupe) et OT_DIAG_TO_USER (personnes) ne se combinent pas" >&2
            return 2
        fi
        status=0
        for person in $TO_USER; do
            printf '%s\n' "$message" | ot send --to-user "$person" || status=$?
        done
        return "$status"
    fi
    set --
    for name in $TO; do set -- "$@" --to "$name"; done
    printf '%s\n' "$message" | ot send "$@"
}

lock() {
    exec 9>"$STATE/lock"
    flock 9
}

unlock() {
    flock -u 9
}

report() {
    lock
    source=$(power_source)
    message=$(build_report)
    send "$message"
    echo "$source" >"$STATE/power.last"
    unlock
}

power() {
    sleep "$DEBOUNCE"
    while :; do
        lock
        source=$(power_source)
        last=$(attr "$STATE" power.last)
        if [ "$source" = "$last" ]; then
            unlock
            return
        fi
        sent=$(attr "$STATE" power.sent)
        wait=$((${sent:-0} + MIN_INTERVAL - $(date +%s)))
        if [ "$wait" -gt 0 ]; then
            unlock
            sleep "$wait"
            continue
        fi
        message=$(build_report)
        send "$message"
        echo "$source" >"$STATE/power.last"
        date +%s >"$STATE/power.sent"
        unlock
    done
}

case ${1:-} in
    report) report ;;
    power) power ;;
    preview) build_report ;;
    *)
        echo "usage : $0 report|power|preview" >&2
        exit 2
        ;;
esac
