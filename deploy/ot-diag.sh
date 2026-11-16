#!/bin/sh
# Diagnostic du relais envoyé par OldTown (note 0025) : alimentation, batterie
# et erreurs disque relevées par le noyau.
#
#   ot-diag.sh report    rapport complet (ot-diag.timer : 00:00 et 12:00)
#   ot-diag.sh power     changement d'alimentation (règle udev) : attend que
#                        l'état se stabilise, n'envoie que s'il a changé depuis
#                        le dernier envoi, au plus un envoi par intervalle
#   ot-diag.sh preview   affiche le rapport sans l'envoyer
#
# Envoi par `ot send` sous l'identité dédiée (OT_HOME), à OT_DIAG_TO (noms de
# membres séparés par des espaces ; vide : tous les autres membres).
set -eu

STATE=${OT_DIAG_STATE:-${STATE_DIRECTORY:-/var/lib/ot-diag}}
SYSFS=${OT_DIAG_SYSFS:-/sys/class/power_supply}
TO=${OT_DIAG_TO:-}
HOURS=${OT_DIAG_WINDOW_HOURS:-12}
DEBOUNCE=${OT_DIAG_DEBOUNCE:-30}
MIN_INTERVAL=${OT_DIAG_MIN_INTERVAL:-600}

# Messages du noyau qui signalent un problème de disque ou de système de
# fichiers, appliqués à des lignes « HH:MM message ». Les avertissements bénins
# (ACPI, pare-feu, `nvme … using unchecked data buffer`) ne doivent pas y
# correspondre : voir les tests.
DISK_PATTERN='^[^ ]+ (nvme [^ ]+: .*(timeout|reset|controller is down|abort|failed|error)'
DISK_PATTERN="$DISK_PATTERN|ata[0-9.]+: .*(exception|failed command|error|hard resetting))"
DISK_PATTERN="$DISK_PATTERN|I/O error|medium error|unrecovered read error"
DISK_PATTERN="$DISK_PATTERN|EXT4-fs error|BTRFS (error|critical)|XFS .*(error|corruption)"
DISK_PATTERN="$DISK_PATTERN|remounting filesystem read-only"

# Fichier d'une source d'alimentation, vide s'il n'existe pas.
attr() {
    if [ -r "$1/$2" ]; then cat "$1/$2"; fi
}

# Source d'alimentation actuelle : `mains` ou `battery`. Les sources du
# système (secteur, USB-C) font foi ; sans elles, l'état de la batterie.
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

# Dossier de la batterie du système (pas celle d'une souris), vide sinon.
battery_dir() {
    for d in "$SYSFS"/*; do
        [ "$(attr "$d" type)" = Battery ] || continue
        [ "$(attr "$d" scope)" != Device ] || continue
        echo "$d"
        return
    done
}

# « Alimentation : secteur, 59 % (en charge) » : source, niveau et état de
# la batterie, comme percentage et state de `upower -i`. Batterie en attente
# de charge (seuil de charge atteint) : pas d'état, il n'apprend rien.
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

# Erreurs disque des $HOURS dernières heures, dans le journal du noyau.
disk_lines() {
    # Le noyau écrit toujours dans le journal au démarrage : un journal vide
    # veut dire qu'il est illisible (groupe systemd-journal manquant), pas
    # qu'il n'y a rien à signaler.
    if [ -z "$(journalctl -k -b -q --no-pager -n 1 -o cat)" ]; then
        echo "SSD : journal du noyau illisible, diagnostic impossible"
        return
    fi
    # _TRANSPORT=kernel plutôt que -k, limité au démarrage en cours : une
    # erreur disque suivie d'un plantage et d'un redémarrage doit rester visible.
    log=$(journalctl _TRANSPORT=kernel -q --no-pager -o short-iso -p warning --since "-${HOURS}h")
    # « 2026-10-12T03:12:45+02:00 hôte kernel: message » → « 03:12 message ».
    # grep rend 1 quand rien ne correspond : ce n'est pas une erreur.
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
    # Les trois messages distincts les plus récents, tronqués.
    printf '%s\n' "$alerts" | cut -d ' ' -f 2- | tac | awk '!seen[$0]++' | head -n 3 | tac \
        | cut -c 1-100 | sed 's/^/- /'
}

# Rapport : une ligne d'alimentation, puis l'état du disque.
build_report() {
    power_line
    disk_lines
}

send() {
    message=$1
    set --
    for name in $TO; do set -- "$@" --to "$name"; done
    printf '%s\n' "$message" | ot send "$@"
}

# Un seul envoi à la fois : deux `ot send` de la même identité se couperaient
# (note 0009).
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
    # Assignation à part : une erreur pendant le rapport arrête le script au
    # lieu d'envoyer un rapport tronqué.
    message=$(build_report)
    send "$message"
    # Le rapport fait connaître l'alimentation : un événement qui n'y change
    # rien ne donnera pas de message.
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
            # Trop tôt : attendre, puis relire l'état, qui a pu revenir.
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
