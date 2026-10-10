-- Schéma initial du relais. Horodatages en millisecondes depuis l'époque Unix.

CREATE TABLE devices (
    id           TEXT PRIMARY KEY NOT NULL,      -- 'dev_' + ULID
    name         TEXT NOT NULL,
    public_key   BLOB NOT NULL UNIQUE,           -- Ed25519, 32 octets
    created_at   INTEGER NOT NULL,
    last_seen_at INTEGER,
    revoked_at   INTEGER
) STRICT;

-- Un nom n'est unique que parmi les appareils actifs ; les révoqués restent pour l'audit.
CREATE UNIQUE INDEX devices_active_name ON devices (name) WHERE revoked_at IS NULL;

CREATE TABLE messages (
    -- AUTOINCREMENT : un seq n'est jamais réutilisé, même après purge.
    seq        INTEGER PRIMARY KEY AUTOINCREMENT,
    msg_id     TEXT NOT NULL UNIQUE,
    sender_id  TEXT NOT NULL REFERENCES devices (id),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    size       INTEGER NOT NULL,
    envelope   BLOB NOT NULL                     -- enveloppe JSON, opaque pour le relais
) STRICT;

CREATE INDEX messages_expires_at ON messages (expires_at);

CREATE TABLE deliveries (
    seq          INTEGER NOT NULL REFERENCES messages (seq) ON DELETE CASCADE,
    device_id    TEXT NOT NULL REFERENCES devices (id),
    delivered_at INTEGER,
    acked_at     INTEGER,
    PRIMARY KEY (seq, device_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX deliveries_pending ON deliveries (device_id, acked_at);
