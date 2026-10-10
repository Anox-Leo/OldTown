-- Jetons d'appairage (étape 2). Le jeton n'est jamais stocké en clair : seul
-- son SHA-256 l'est. Horodatages en millisecondes depuis l'époque Unix.

CREATE TABLE pairings (
    token_hash BLOB PRIMARY KEY NOT NULL,                -- SHA-256 du jeton, 32 octets
    sponsor_id TEXT NOT NULL REFERENCES devices (id),    -- appareil qui a créé le jeton
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    used_at    INTEGER,
    device_id  TEXT REFERENCES devices (id)              -- appareil créé avec le jeton
) STRICT, WITHOUT ROWID;

CREATE INDEX pairings_expires_at ON pairings (expires_at);
