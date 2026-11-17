#!/bin/sh
# Sauvegarde de ce qui ne se reconstruit pas sur le relais : personnes,
# appareils enregistrés, liens, services coupés et journaux de roster (étape
# 10). Ni les messages, ni les fichiers en transit, ni les jetons
# d'appairage ou d'invitation, qui n'ont rien à faire dans une sauvegarde.
# Produit des INSERT rejouables sur une base neuve, dans l'ordre des clés
# étrangères : créer la base (ot-server user list, relais arrêté), puis
# sqlite3 <base> < devices-<date>.sql.
set -eu

DB=${OT_DB:-/var/lib/ot-relay/ot.db}
DIR=${OT_BACKUP_DIR:-/var/lib/ot-relay/backups}
KEEP=${OT_BACKUP_KEEP:-14}

mkdir -p "$DIR"
out="$DIR/devices-$(date -u +%Y%m%dT%H%M%SZ).sql"
# Une base d'avant l'étape 10 n'a que `devices`.
if sqlite3 -readonly "$DB" "SELECT 1 FROM sqlite_master WHERE name = 'users'" | grep -q 1; then
  {
    # La personne `owner` créée à l'ouverture d'une base neuve est remplacée.
    echo "DELETE FROM users WHERE NOT EXISTS (SELECT 1 FROM devices WHERE devices.user_id = users.id);"
    sqlite3 -readonly "$DB" \
      '.mode insert users' 'SELECT * FROM users ORDER BY created_at, rowid;' \
      '.mode insert devices' 'SELECT * FROM devices ORDER BY created_at, rowid;' \
      '.mode insert links' 'SELECT * FROM links ORDER BY created_at;' \
      '.mode insert muted_services' 'SELECT * FROM muted_services;' \
      '.mode insert roster_log' 'SELECT * FROM roster_log ORDER BY user_id, seq;'
  } > "$out.tmp"
else
  sqlite3 -readonly "$DB" '.mode insert devices' 'SELECT * FROM devices ORDER BY created_at;' > "$out.tmp"
fi
mv "$out.tmp" "$out"

# Ne garde que les $KEEP plus récentes.
ls -1t "$DIR"/devices-*.sql | tail -n +"$((KEEP + 1))" | while read -r old; do rm -f -- "$old"; done
