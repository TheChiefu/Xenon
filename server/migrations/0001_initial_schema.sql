-- Initial schema.
--
-- Connection settings live in SqliteConnectOptions.
-- Timestamps are Unix milliseconds, set in Rust by now_ms().
-- Length CHECKs are structural. Maximums live in the server config.


-- Content addressed: sha256 is both the dedup key and the path on disk.
-- filename is the first uploader's name. mime is sniffed from the bytes.
CREATE TABLE files (
    id          BLOB PRIMARY KEY CHECK (length(id) = 16),
    sha256      BLOB NOT NULL UNIQUE CHECK (length(sha256) = 32),
    filename    TEXT NOT NULL CHECK (length(filename) BETWEEN 1 AND 255), -- Served as the client's download name, where 255 is the file system limit
    mime        TEXT NOT NULL CHECK (length(mime) BETWEEN 3 AND 255),
    byte_size   INTEGER NOT NULL CHECK (byte_size > 0),
    created_at  INTEGER NOT NULL
) STRICT;

-- username is restricted to lowercase, so UNIQUE rejects 'Alice' while 'alice'
-- exists. The email CHECK serves one_email the same way.
CREATE TABLE users (
    id              BLOB PRIMARY KEY CHECK (length(id) = 16),
    username        TEXT NOT NULL UNIQUE
                    CHECK (username NOT GLOB '*[^a-z0-9_-]*' AND length(username) >= 1),
    display_name    TEXT NOT NULL CHECK (length(display_name) >= 1),
    description     TEXT NOT NULL DEFAULT '',
    avatar_file_id  BLOB REFERENCES files(id),
    banner_file_id  BLOB REFERENCES files(id),
    password_hash   TEXT,
    global_role     INTEGER NOT NULL CHECK (global_role IN (0, 1, 2, 3)),
    preferred_status INTEGER NOT NULL DEFAULT 0 CHECK (preferred_status IN (0, 1, 2, 3)),
    email           TEXT CHECK (email IS NULL OR
                        (email = lower(email) AND length(email) BETWEEN 3 AND 254)),
    created_at      INTEGER NOT NULL,
    deleted_at      INTEGER,
    CHECK (deleted_at IS NULL OR deleted_at >= created_at)
) STRICT;

CREATE UNIQUE INDEX one_owner ON users(global_role) WHERE global_role = 0 AND deleted_at IS NULL;
CREATE UNIQUE INDEX one_email ON users(email) WHERE email IS NOT NULL AND deleted_at IS NULL;

CREATE TABLE linked_accounts (
    user_id          BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    platform         INTEGER NOT NULL CHECK (platform IN (0, 1, 2)),
    platform_user_id TEXT NOT NULL CHECK (length(platform_user_id) BETWEEN 1 AND 128),
    platform_handle  TEXT CHECK (platform_handle IS NULL OR length(platform_handle) <= 128),
    linked_at        INTEGER NOT NULL,
    PRIMARY KEY (user_id, platform),
    UNIQUE (platform, platform_user_id)
) STRICT;

CREATE TABLE sessions (
    token_hash  BLOB PRIMARY KEY CHECK (length(token_hash) = 32),
    user_id     BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL CHECK (expires_at > created_at),
    revoked_at  INTEGER CHECK (revoked_at IS NULL OR revoked_at >= created_at)
) STRICT;

CREATE INDEX sessions_user ON sessions(user_id);
CREATE INDEX sessions_expiry ON sessions(expires_at) WHERE revoked_at IS NULL;

-- Visibility:
-- - 0: Public
-- - 1: Locked
-- - 2: Hidden
--
-- default_permissions has no DEFAULT: creation must state it. 0 is a read-only
-- room, and the value is copied into room_access.permissions when a member joins.
CREATE TABLE rooms (
    id                  BLOB PRIMARY KEY CHECK (length(id) = 16),
    name                TEXT NOT NULL,
    visibility          INTEGER NOT NULL CHECK (visibility IN (0, 1, 2)),
    default_permissions INTEGER NOT NULL CHECK (default_permissions >= 0),
    created_at          INTEGER NOT NULL,
    -- Incremented on edit and tombstone
    mutation_seq        INTEGER NOT NULL DEFAULT 0 CHECK (mutation_seq >= 0)
) STRICT;

-- The directory query:
-- Hidden rooms are absent from the index and id is a UUIDv7 (byte order is creation order)
-- of which the directory pages on with `id > ?`.
--
-- Queries must filter with `visibility IN (0, 1)` to match this predicate.
CREATE INDEX rooms_directory ON rooms(id) WHERE visibility IN (0, 1);

-- A row grants read access to the room
--
-- Notify:
-- - 0: None
-- - 1: Mentions
-- - 2: All
--
-- granted_at holds join time, and orders the member list
CREATE TABLE room_access (
    room_id     BLOB NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    user_id     BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    permissions INTEGER NOT NULL CHECK (permissions >= 0),
    notify      INTEGER NOT NULL DEFAULT 0 CHECK (notify IN (0, 1, 2)),
    granted_at  INTEGER NOT NULL,
    PRIMARY KEY (room_id, user_id)
) STRICT;

CREATE INDEX room_access_user ON room_access(user_id);

-- Pending invitations to Locked and Hidden rooms
-- A row means invited, not joined
CREATE TABLE room_invites (
    room_id     BLOB NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    user_id     BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    invited_by  BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER CHECK (expires_at IS NULL OR expires_at > created_at),
    PRIMARY KEY (room_id, user_id)
) STRICT;

CREATE INDEX room_invites_user ON room_invites(user_id);

-- Room scoped bans:
-- created_by is a plain REFERENCES, never CASCADE (a cascade lifts every ban an issuer made)
CREATE TABLE room_bans (
    room_id     BLOB NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    user_id     BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_by  BLOB REFERENCES users(id),
    reason      TEXT CHECK (reason IS NULL OR length(reason) BETWEEN 1 AND 500),
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER CHECK (expires_at IS NULL OR expires_at > created_at),
    PRIMARY KEY (room_id, user_id)
) STRICT;

CREATE TABLE read_state (
    user_id       BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    room_id       BLOB NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    last_read_seq INTEGER NOT NULL CHECK (last_read_seq >= 0),
    PRIMARY KEY (user_id, room_id)
) STRICT;

CREATE TABLE messages (
    seq           INTEGER PRIMARY KEY AUTOINCREMENT,
    id            BLOB NOT NULL UNIQUE CHECK (length(id) = 16),
    room_id       BLOB NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    author_id     BLOB NOT NULL REFERENCES users(id),
    body          TEXT CHECK (body IS NULL OR length(body) >= 1),
    client_nonce  BLOB NOT NULL CHECK (length(client_nonce) = 16),
    created_at    INTEGER NOT NULL,
    edited_at     INTEGER CHECK (edited_at IS NULL OR edited_at >= created_at),
    deleted_at    INTEGER CHECK (deleted_at IS NULL OR deleted_at >= created_at),
    CHECK (deleted_at IS NULL OR body IS NULL)
) STRICT;

CREATE INDEX messages_room ON messages(room_id, seq);
CREATE INDEX messages_room_live ON messages(room_id, seq) WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX message_nonce ON messages(author_id, client_nonce);

CREATE TRIGGER messages_author_is_member BEFORE INSERT ON messages
WHEN NOT EXISTS (
    SELECT 1 FROM room_access a
    WHERE a.room_id = new.room_id AND a.user_id = new.author_id
)
BEGIN SELECT RAISE(ABORT, 'author is not a member of this room');
END;

CREATE TRIGGER messages_author_live BEFORE INSERT ON messages
WHEN (SELECT deleted_at FROM users WHERE id = new.author_id) IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'author is deleted');
END;

CREATE VIRTUAL TABLE messages_fts USING fts5(
    body, content = 'messages', content_rowid = 'seq', tokenize = 'trigram'
);

CREATE TRIGGER messages_fts_insert AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, body) VALUES (new.seq, new.body);
END;

CREATE TRIGGER messages_fts_delete AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, body) VALUES ('delete', old.seq, old.body);
END;

CREATE TRIGGER messages_fts_update AFTER UPDATE OF body ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, body) VALUES ('delete', old.seq, old.body);
    INSERT INTO messages_fts(rowid, body) VALUES (new.seq, new.body);
END;

-- ordinal is the display order
CREATE TABLE message_attachments (
    message_id  BLOB NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    file_id     BLOB NOT NULL REFERENCES files(id),
    ordinal     INTEGER NOT NULL CHECK (ordinal >= 0),
    spoiler     INTEGER NOT NULL DEFAULT 0 CHECK (spoiler IN (0, 1)),
    PRIMARY KEY (message_id, ordinal),
    UNIQUE (message_id, file_id)
) STRICT;

CREATE INDEX message_attachments_file ON message_attachments(file_id);

-- A user's library of files
CREATE TABLE user_files (
    user_id   BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    file_id   BLOB NOT NULL REFERENCES files(id),
    added_at  INTEGER NOT NULL,
    PRIMARY KEY (user_id, file_id)
) STRICT;

CREATE INDEX user_files_file ON user_files(file_id);

-- Server registration invites
CREATE TABLE invites (
    code        TEXT PRIMARY KEY
                CHECK (code NOT GLOB '*[^A-Z0-9]*' AND length(code) BETWEEN 12 AND 64),
    created_by  BLOB NOT NULL REFERENCES users(id),
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER CHECK (expires_at IS NULL OR expires_at > created_at),
    max_uses    INTEGER CHECK (max_uses IS NULL OR max_uses > 0),
    uses        INTEGER NOT NULL DEFAULT 0 CHECK (uses >= 0),
    revoked_at  INTEGER CHECK (revoked_at IS NULL OR revoked_at >= created_at),
    CHECK (max_uses IS NULL OR uses <= max_uses)
) STRICT;

-- The VAPID public key browsers subscribe against. Browser subscriptions
-- themselves (push_subscriptions) are not Xenon's to keep: only the push
-- sidecar reads them, so they live in its own store instead.
CREATE TABLE push_keys (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    public_key  BLOB NOT NULL CHECK (length(public_key) = 65),
    created_at  INTEGER NOT NULL
) STRICT;