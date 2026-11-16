-- Fichiers chiffrés déposés par morceaux (étape 5, note 0026). Le relais ne
-- connaît que la géométrie du chiffré ; les morceaux sont sur disque, un
-- fichier par morceau. Horodatages en millisecondes depuis l'époque Unix.

CREATE TABLE blobs (
    id           TEXT PRIMARY KEY NOT NULL,             -- 'blob_' + ULID
    owner_id     TEXT NOT NULL REFERENCES devices (id), -- seul à déposer
    size         INTEGER NOT NULL,                      -- taille chiffrée totale
    chunk_size   INTEGER NOT NULL,                      -- taille d'un morceau chiffré
    chunks       INTEGER NOT NULL,
    created_at   INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL,
    completed_at INTEGER                                -- tous les morceaux déposés
) STRICT;

CREATE INDEX blobs_expires_at ON blobs (expires_at);

-- Morceaux présents sur disque (écrits puis renommés avant d'être notés ici).
CREATE TABLE blob_chunks (
    blob_id TEXT NOT NULL REFERENCES blobs (id) ON DELETE CASCADE,
    idx     INTEGER NOT NULL,
    PRIMARY KEY (blob_id, idx)
) STRICT, WITHOUT ROWID;

CREATE TABLE blob_recipients (
    blob_id   TEXT NOT NULL REFERENCES blobs (id) ON DELETE CASCADE,
    device_id TEXT NOT NULL REFERENCES devices (id),
    done_at   INTEGER,                                  -- tout retiré
    PRIMARY KEY (blob_id, device_id)
) STRICT, WITHOUT ROWID;
