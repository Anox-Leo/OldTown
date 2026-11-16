#!/bin/sh
# Sauvegarde des appareils enregistrés sur le relais (pas des messages, qui
# sont en clair à l'étape 1 et n'ont rien à faire dans une sauvegarde).
# Produit des INSERT rejouables sur une base neuve : restaurer avec sqlite3 <base> < devices-<date>.sql (relais arrêté).
set -eu

DB=${OT_DB:-/var/lib/ot-relay/ot.db}
DIR=${OT_BACKUP_DIR:-/var/lib/ot-relay/backups}
KEEP=${OT_BACKUP_KEEP:-14}

mkdir -p "$DIR"
out="$DIR/devices-$(date -u +%Y%m%dT%H%M%SZ).sql"
sqlite3 -readonly "$DB" '.mode insert devices' 'SELECT * FROM devices ORDER BY created_at;' > "$out.tmp"
mv "$out.tmp" "$out"

# Ne garde que les $KEEP plus récentes.
ls -1t "$DIR"/devices-*.sql | tail -n +"$((KEEP + 1))" | while read -r old; do rm -f -- "$old"; done
