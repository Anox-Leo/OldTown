# Image du relais OldTown (étape 8e, note 0038), construite depuis la racine
# du dépôt : docker build -f deploy/docker/Dockerfile .
#
# Le relais écoute en clair sur le réseau interne ; Caddy termine le TLS
# devant lui (deploy/docker/compose.yaml).

FROM rust:1.99-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p ot-server \
    && install -d -m 0700 /out/data

# Sans shell ni gestionnaire de paquets ; utilisateur sans privilèges.
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /src/target/release/ot-server /usr/local/bin/ot-server
COPY --from=build --chown=nonroot:nonroot /out/data /var/lib/ot-relay
ENV OT_DB=/var/lib/ot-relay/ot.db \
    OT_LISTEN=0.0.0.0:7878 \
    OT_ADMIN_LISTEN=0.0.0.0:7879 \
    OT_LOG_FORMAT=json
VOLUME /var/lib/ot-relay
EXPOSE 7878
ENTRYPOINT ["/usr/local/bin/ot-server"]
CMD ["serve"]
