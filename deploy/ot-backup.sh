#!/bin/sh
set -eu

DB=${OT_DB:-/var/lib/ot-relay/ot.db}
DIR=${OT_BACKUP_DIR:-/var/lib/ot-relay/backups}
KEEP=${OT_BACKUP_KEEP:-14}

mkdir -p "$DIR"
out="$DIR/devices-$(date -u +%Y%m%dT%H%M%SZ).sql"
if sqlite3 -readonly "$DB" "SELECT 1 FROM sqlite_master WHERE name = 'users'" | grep -q 1; then
  {
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

ls -1t "$DIR"/devices-*.sql | tail -n +"$((KEEP + 1))" | while read -r old; do rm -f -- "$old"; done
