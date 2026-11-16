#!/bin/sh
# Tests de deploy/ot-diag.sh, sans privilèges ni vrai relais : sources
# d'alimentation, journal du noyau et `ot` sont simulés.
#
#   sh deploy/tests/ot-diag-test.sh
set -eu

SCRIPT=$(cd "$(dirname "$0")/.." && pwd)/ot-diag.sh
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT

failures=0
fail() {
    echo "ÉCHEC : $1" >&2
    failures=$((failures + 1))
}

# Vérifie que $2 contient la ligne exacte, ou le motif, $3.
expect_line() {
    printf '%s\n' "$2" | grep -q -x -F -e "$3" || fail "$1 : ligne attendue « $3 » dans :
$2"
}
expect_match() {
    printf '%s\n' "$2" | grep -q -E -e "$3" || fail "$1 : motif attendu « $3 » dans :
$2"
}
expect_no_match() {
    if printf '%s\n' "$2" | grep -q -E -e "$3"; then fail "$1 : motif inattendu « $3 » dans :
$2"; fi
}

# Faux journalctl : la sonde (-n 1) renvoie $T/probe, le reste $T/journal.
mkdir -p "$T/bin"
cat >"$T/bin/journalctl" <<'EOF'
#!/bin/sh
case " $* " in
    *" -n 1 "*) cat "$OT_TEST_DIR/probe" ;;
    *) printf '%s\n' "$*" >"$OT_TEST_DIR/journal.args"; cat "$OT_TEST_DIR/journal" ;;
esac
EOF
# Faux ot : garde les arguments et le texte de chaque envoi.
cat >"$T/bin/ot" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >>"$OT_TEST_DIR/ot.args"
cat >>"$OT_TEST_DIR/ot.out"
exit "${OT_TEST_OT_STATUS:-0}"
EOF
chmod +x "$T/bin/journalctl" "$T/bin/ot"

export PATH="$T/bin:$PATH" OT_TEST_DIR="$T"
export OT_DIAG_STATE="$T/state" OT_DIAG_SYSFS="$T/sys"
export OT_DIAG_DEBOUNCE=0 OT_DIAG_MIN_INTERVAL=0 OT_DIAG_TO=android

# Remet un laptop sur secteur, batterie à 59 % en attente de charge, journal
# sans alerte.
reset() {
    rm -rf "$T/sys" "$T/state" "$T/ot.args" "$T/ot.out"
    mkdir -p "$T/state"
    supply ACAD type=Mains online=1
    supply BAT1 type=Battery status="Not charging" capacity=59
    supply ucsi-source-psy-USBC000:001 type=USB scope=System online=0
    echo "Linux version 7.0.0" >"$T/probe"
    : >"$T/journal"
}

# supply NOM attribut=valeur…
supply() {
    dir="$T/sys/$1"
    shift
    mkdir -p "$dir"
    for kv in "$@"; do printf '%s\n' "${kv#*=}" >"$dir/${kv%%=*}"; done
}

kernel() {
    printf '2026-10-12T%s+02:00 relais kernel: %s\n' "$1" "$2" >>"$T/journal"
}

sent_count() {
    if [ -f "$T/ot.args" ]; then wc -l <"$T/ot.args" | tr -d ' '; else echo 0; fi
}

# --- Rapport sur secteur, sans alerte.
reset
out=$(sh "$SCRIPT" preview)
[ "$out" = "Alimentation : secteur, 59 %
SSD : aucune alerte sur 12 h" ] || fail "rapport : deux lignes attendues, reçu :
$out"
expect_match "fenêtre du journal" "$(cat "$T/journal.args")" '--since -12h'
# Tous les démarrages des 12 h, pas seulement le dernier (-k).
expect_match "journal sur plusieurs démarrages" "$(cat "$T/journal.args")" '^_TRANSPORT=kernel '
expect_no_match "journal sur plusieurs démarrages" "$(cat "$T/journal.args")" '(^| )-k( |$)'

# --- Batterie qui se décharge.
reset
supply ACAD online=0
supply BAT1 status=Discharging capacity=41
out=$(sh "$SCRIPT" preview)
expect_line "décharge" "$out" "Alimentation : batterie, 41 % (en décharge)"

# --- Sans batterie.
reset
rm -r "$T/sys/BAT1"
out=$(sh "$SCRIPT" preview)
expect_line "sans batterie" "$out" "Alimentation : secteur, pas de batterie"

# --- Seule source système : l'USB-C ; une souris (scope Device) ne compte pas.
reset
rm -r "$T/sys/ACAD"
supply ucsi-source-psy-USBC000:001 online=1
supply hid-souris-battery type=Battery scope=Device capacity=5 status=Discharging
out=$(sh "$SCRIPT" preview)
expect_line "USB-C" "$out" "Alimentation : secteur, 59 %"

# --- Avertissements bénins : pas de fausse alerte.
reset
kernel 01:00:00 "[UFW BLOCK] IN=wlan0 OUT= SRC=192.0.2.1 DST=224.0.0.1 LEN=28 PROTO=2"
kernel 01:00:01 "ACPI Error: AE_ALREADY_EXISTS, During name lookup/catalog (20250807/psobject-220)"
kernel 01:00:02 "nvme nvme0: using unchecked data buffer"
kernel 01:00:03 "ucsi_acpi USBC000:00: UCSI_GET_PDOS failed (-5)"
kernel 01:00:04 "EXT4-fs (nvme0n1p2): mounted filesystem with ordered data mode"
out=$(sh "$SCRIPT" preview)
expect_line "bénin" "$out" "SSD : aucune alerte sur 12 h"

# --- Erreurs disque : compte, heure de la dernière, messages distincts.
reset
kernel 03:10:00 "nvme nvme0: I/O 12 QID 3 timeout, aborting"
kernel 03:11:00 "I/O error, dev nvme0n1, sector 1234 op 0x0:(READ) flags 0x0 phys_seg 1 prio class 2"
kernel 03:12:00 "nvme nvme0: I/O 12 QID 3 timeout, aborting"
kernel 03:12:30 "[UFW BLOCK] IN=wlan0 OUT= SRC=192.0.2.1"
out=$(sh "$SCRIPT" preview)
expect_line "erreurs" "$out" "SSD : 3 erreurs détectées sur 12 h, dernière à 03:12 :"
expect_line "erreurs" "$out" "- I/O error, dev nvme0n1, sector 1234 op 0x0:(READ) flags 0x0 phys_seg 1 prio class 2"
expect_line "erreurs" "$out" "- nvme nvme0: I/O 12 QID 3 timeout, aborting"
[ "$(printf '%s\n' "$out" | grep -c '^- ')" = 2 ] || fail "erreurs : deux messages distincts attendus"

reset
kernel 04:05:00 "EXT4-fs error (device nvme0n1p2): ext4_find_entry:1683: inode #2: comm ls: reading directory lblock 0"
out=$(sh "$SCRIPT" preview)
expect_line "une erreur" "$out" "SSD : 1 erreur détectée sur 12 h, à 04:05 :"

# --- Journal illisible : le dire, pas « aucune alerte ».
reset
: >"$T/probe"
out=$(sh "$SCRIPT" preview)
expect_line "illisible" "$out" "SSD : journal du noyau illisible, diagnostic impossible"

# --- Envoi du rapport : destinataires, état d'alimentation retenu.
reset
OT_DIAG_TO="android mac" sh "$SCRIPT" report 2>/dev/null
expect_line "envoi" "$(cat "$T/ot.args")" "send --to android --to mac"
expect_line "envoi" "$(cat "$T/ot.out")" "Alimentation : secteur, 59 %"
expect_line "envoi" "$(cat "$T/state/power.last")" "mains"

reset
OT_DIAG_TO="" sh "$SCRIPT" report 2>/dev/null
expect_line "tous les membres" "$(cat "$T/ot.args")" "send"

# --- Changements d'alimentation.
reset
sh "$SCRIPT" report 2>/dev/null
sh "$SCRIPT" power
[ "$(sent_count)" = 1 ] || fail "alimentation inchangée : aucun envoi attendu"

supply ACAD online=0
sh "$SCRIPT" power
[ "$(sent_count)" = 2 ] || fail "passage sur batterie : un envoi attendu"
expect_line "passage sur batterie" "$(tail -n 2 "$T/ot.out" | head -n 1)" "Alimentation : batterie, 59 %"
sh "$SCRIPT" power
[ "$(sent_count)" = 2 ] || fail "batterie, deuxième événement : aucun envoi attendu"

supply ACAD online=1
OT_DIAG_MIN_INTERVAL=1 sh "$SCRIPT" power
[ "$(sent_count)" = 3 ] || fail "retour sur secteur : un envoi attendu"
expect_line "retour sur secteur" "$(tail -n 2 "$T/ot.out" | head -n 1)" "Alimentation : secteur, 59 %"

# Envoi refusé : erreur, et l'état n'avance pas (nouvel essai par systemd).
supply ACAD online=0
if OT_TEST_OT_STATUS=1 sh "$SCRIPT" power 2>/dev/null; then fail "échec d'envoi : code d'erreur attendu"; fi
expect_line "échec d'envoi" "$(cat "$T/state/power.last")" "mains"

# --- Usage.
if sh "$SCRIPT" 2>/dev/null; then fail "sans argument : code d'erreur attendu"; fi

if [ "$failures" -gt 0 ]; then
    echo "$failures échec(s)" >&2
    exit 1
fi
echo "ot-diag : tous les tests passent"
