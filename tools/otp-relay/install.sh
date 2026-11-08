#!/bin/sh
# Installe otp-relay pour l'utilisateur courant (sans root) :
#   code       → ~/.local/lib/otp-relay
#   commande   → ~/.local/bin/otp-relay
#   config     → ~/.config/otp-relay/config.toml (créée seulement si absente)
#   service    → ~/.config/systemd/user/otp-relay.service (non activé)
set -eu

SRC=$(cd "$(dirname "$0")" && pwd)
LIB="$HOME/.local/lib/otp-relay"
BIN="$HOME/.local/bin"
CONF="${XDG_CONFIG_HOME:-$HOME/.config}/otp-relay"
UNITS="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

python3 -c 'import sys; sys.exit(sys.version_info < (3, 14))' \
    || { echo "Python >= 3.14 requis (IDLE natif dans imaplib)" >&2; exit 1; }

rm -rf "$LIB.new"
mkdir -p "$LIB.new" "$BIN" "$UNITS"
cp -r "$SRC/otp_relay" "$SRC/README.md" "$LIB.new/"
find "$LIB.new" -name __pycache__ -prune -exec rm -rf {} +
rm -rf "$LIB" && mv "$LIB.new" "$LIB"

cat > "$BIN/otp-relay" <<EOF
#!/bin/sh
PYTHONPATH="$LIB" exec python3 -m otp_relay "\$@"
EOF
chmod 755 "$BIN/otp-relay"

install -d -m 700 "$CONF"
if [ ! -e "$CONF/config.toml" ]; then
    install -m 600 "$SRC/contrib/config.example.toml" "$CONF/config.toml"
    echo "config créée : $CONF/config.toml (à compléter)"
fi

install -m 644 "$SRC/contrib/otp-relay.service" "$UNITS/otp-relay.service"
systemctl --user daemon-reload

echo "installé dans $LIB"
echo "suite : voir README.md, section « Mise en service »"
