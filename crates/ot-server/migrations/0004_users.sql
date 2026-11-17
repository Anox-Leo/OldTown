-- Personnes, réseau et invitations (étape 10, note 0052). Horodatages en
-- millisecondes depuis l'époque Unix. La personne `admin` qui reprend les
-- appareils existants est créée par `Store::open` (un ULID ne se tire pas
-- en SQL) : après l'ouverture, `devices.user_id` n'est jamais NULL.

CREATE TABLE users (
    id         TEXT PRIMARY KEY NOT NULL,                -- 'usr_' + ULID
    name       TEXT NOT NULL,
    role       TEXT NOT NULL CHECK (role IN ('admin', 'member', 'service')),
    state      TEXT NOT NULL CHECK (state IN ('pending', 'active', 'revoked')),
    cert       BLOB,                                     -- SignedBlob JSON, opaque ; NULL tant que non certifiée
    created_at INTEGER NOT NULL,
    revoked_at INTEGER
) STRICT;

-- Un nom n'est unique que parmi les personnes non révoquées ; un seul admin.
CREATE UNIQUE INDEX users_active_name ON users (name) WHERE state != 'revoked';
CREATE UNIQUE INDEX users_one_admin ON users (role) WHERE role = 'admin' AND state != 'revoked';

ALTER TABLE devices ADD COLUMN user_id TEXT REFERENCES users (id);
ALTER TABLE devices ADD COLUMN kind TEXT;                -- NULL : deviné au nom par les clients
ALTER TABLE devices ADD COLUMN reachable INTEGER NOT NULL DEFAULT 1;
ALTER TABLE devices ADD COLUMN is_default INTEGER NOT NULL DEFAULT 0;
ALTER TABLE devices ADD COLUMN features TEXT NOT NULL DEFAULT '';  -- séparées par ','

-- Le nom d'un appareil n'est plus unique que dans sa personne.
DROP INDEX devices_active_name;
CREATE UNIQUE INDEX devices_active_name ON devices (user_id, name) WHERE revoked_at IS NULL;
CREATE INDEX devices_user ON devices (user_id);

-- Lien « from peut écrire à to ». Un lien entre personnes = deux lignes ;
-- un service n'a que le sens service → personne (D41).
CREATE TABLE links (
    from_user  TEXT NOT NULL REFERENCES users (id),
    to_user    TEXT NOT NULL REFERENCES users (id),
    created_at INTEGER NOT NULL,
    PRIMARY KEY (from_user, to_user),
    CHECK (from_user != to_user)
) STRICT, WITHOUT ROWID;

CREATE INDEX links_to ON links (to_user);

-- Services coupés par leur destinataire : ils ne reviennent que s'il les
-- rouvre lui-même, même si l'admin repose le lien.
CREATE TABLE muted_services (
    user_id    TEXT NOT NULL REFERENCES users (id),
    service_id TEXT NOT NULL REFERENCES users (id),
    PRIMARY KEY (user_id, service_id)
) STRICT, WITHOUT ROWID;

-- Invitations : comme `pairings`, jamais le jeton en clair. Une invitation
-- utilisée relie ses deux appareils, même sans lien entre les personnes,
-- jusqu'à ce que l'invité publie son journal (ou qu'elle expire).
CREATE TABLE invitations (
    token_hash BLOB PRIMARY KEY NOT NULL,                -- SHA-256 du jeton, 32 octets
    user_id    TEXT NOT NULL REFERENCES users (id),
    created_by TEXT NOT NULL REFERENCES devices (id),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    used_at    INTEGER,
    device_id  TEXT REFERENCES devices (id)
) STRICT, WITHOUT ROWID;

CREATE INDEX invitations_expires_at ON invitations (expires_at);
CREATE INDEX invitations_user ON invitations (user_id);

-- Journal du roster de chaque personne, en ajout seul (vidé seulement à la
-- réinvitation et par `ot-server admin reset`).
CREATE TABLE roster_log (
    user_id TEXT NOT NULL REFERENCES users (id),
    seq     INTEGER NOT NULL,                            -- 1, 2, 3…
    entry   BLOB NOT NULL,                               -- SignedBlob JSON, opaque
    PRIMARY KEY (user_id, seq)
) STRICT, WITHOUT ROWID;
