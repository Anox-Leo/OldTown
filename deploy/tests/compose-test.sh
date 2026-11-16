#!/bin/sh
# Essai du relais public (deploy/docker, note 0038) : construit l'image, lance
# le relais et Caddy sur localhost (certificat interne de Caddy, ports hauts),
# puis vérifie le frontal et les limites (note 0036). Demande Docker, curl et
# l'accès aux images caddy:2 et curlimages/curl.
#
#   sh deploy/tests/compose-test.sh
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
env_file=$(mktemp)
printf 'OT_DOMAIN=localhost\nOT_HTTP_PORT=18080\nOT_HTTPS_PORT=18443\nOT_ADMIN_PORT=17879\n' >"$env_file"
compose() {
	docker compose --project-name oldtown-test --env-file "$env_file" \
		-f "$root/deploy/docker/compose.yaml" "$@"
}
cleanup() {
	compose down -v >/dev/null 2>&1 || true
	rm -f "$env_file"
}
trap cleanup EXIT

fail() {
	echo "ÉCHEC : $*" >&2
	compose logs >&2 || true
	exit 1
}

# Code HTTP d'une requête au frontal, en ignorant le certificat interne.
code() {
	curl -sk -o /dev/null -w '%{http_code}' "$@"
}

compose up -d --build

url=https://localhost:18443
i=0
until [ "$(curl -sk "$url/healthz" || true)" = "ok" ]; do
	i=$((i + 1))
	[ "$i" -lt 60 ] || fail "le relais ne répond pas derrière Caddy"
	sleep 1
done
echo "ok : /healthz derrière Caddy"

[ "$(code "$url/metrics")" = 404 ] || fail "/metrics servi sur le port public"
curl -sf http://127.0.0.1:17879/metrics | grep -q '^ot_limited_total' ||
	fail "/metrics absent de l'adresse locale"
echo "ok : /metrics seulement en local"

ws=$(code --http1.1 -m 5 -H 'Connection: Upgrade' -H 'Upgrade: websocket' \
	-H 'Sec-WebSocket-Version: 13' -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
	"$url/v1/ws" || true)
[ "$ws" = 101 ] || fail "upgrade WebSocket : $ws"
echo "ok : WebSocket relayé"

# 30 requêtes non signées épuisent les échecs tolérés de cette IP.
blob=/v1/blobs/blob_01J0000000000000000000000
n=0
while [ "$n" -lt 30 ]; do
	[ "$(code "$url$blob")" = 401 ] || fail "requête non signée acceptée"
	n=$((n + 1))
done
[ "$(code "$url$blob")" = 503 ] || fail "IP non freinée après 30 échecs"
[ "$(code -H 'X-Forwarded-For: 198.51.100.77' "$url$blob")" = 503 ] ||
	fail "X-Forwarded-For forgé par le client cru"
echo "ok : échecs freinés, en-tête du client ignoré"

# Un autre client (autre IP, sur le réseau de la pile) n'est pas touché.
other=$(docker run --rm --network oldtown-test_oldtown curlimages/curl \
	-sk -o /dev/null -w '%{http_code}' --resolve localhost:443:172.31.78.2 \
	"https://localhost$blob")
[ "$other" = 401 ] || fail "un autre client est freiné aussi ($other) : IP de Caddy comptée"
echo "ok : IP réelle de chaque client derrière Caddy"

compose exec -T relay ot-server device list >/dev/null || fail "administration dans le conteneur"
echo "ok : administration (ot-server device list)"
