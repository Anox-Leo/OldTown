FROM rust:1.99-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p ot-server \
    && install -d -m 0700 /out/data

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
