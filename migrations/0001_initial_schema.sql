-- Initial schema. Timestamps are Unix SECONDS.
-- Connection settings (foreign_keys, WAL, synchronous, busy_timeout, secure_delete)
-- are NOT set here. They are per-connection concerns and live in SqliteConnectOptions.
-- Text length CHECKs are outer bounds, not product limits. See data-model.md.

CREATE TABLE files (
    id          BLOB PRIMARY KEY CHECK (length(id) = 16),
    sha256      BLOB NOT NULL UNIQUE CHECK (length(sha256) = 32),
    mime        TEXT NOT NULL CHECK (length(mime) BETWEEN 3 AND 255),
    byte_size   INTEGER NOT NULL CHECK (byte_size >= 0),
    created_at  INTEGER NOT NULL CHECK (created_at BETWEEN 1600000000 AND 4000000000)
) STRICT;

CREATE TABLE users (
    id              BLOB PRIMARY KEY CHECK (length(id) = 16),
    username        TEXT NOT NULL UNIQUE
                    CHECK (username NOT GLOB '*[^a-z0-9_-]*' AND length(username) BETWEEN 1 AND 64),
    display_name    TEXT NOT NULL CHECK (length(display_name) BETWEEN 1 AND 128),
    description     TEXT CHECK (description IS NULL OR length(description) <= 2000),
    avatar_file_id  BLOB REFERENCES files(id),
    banner_file_id  BLOB REFERENCES files(id),
    password_hash   TEXT,
    global_role     INTEGER NOT NULL CHECK (global_role IN (0, 1, 2)),
    status_pref     INTEGER NOT NULL DEFAULT 0 CHECK (status_pref IN (0, 1, 2, 3)),
    email           TEXT CHECK (email IS NULL OR
                        (email = lower(email) AND length(email) BETWEEN 3 AND 254)),
    created_at      INTEGER NOT NULL CHECK (created_at BETWEEN 1600000000 AND 4000000000),
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
    linked_at        INTEGER NOT NULL CHECK (linked_at BETWEEN 1600000000 AND 4000000000),
    PRIMARY KEY (user_id, platform),
    UNIQUE (platform, platform_user_id)
) STRICT;

CREATE TABLE sessions (
    token_hash  BLOB PRIMARY KEY CHECK (length(token_hash) = 32),
    user_id     BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  INTEGER NOT NULL CHECK (created_at BETWEEN 1600000000 AND 4000000000),
    expires_at  INTEGER NOT NULL CHECK (expires_at > created_at),
    revoked_at  INTEGER CHECK (revoked_at IS NULL OR revoked_at >= created_at)
) STRICT;

CREATE INDEX sessions_user ON sessions(user_id);
CREATE INDEX sessions_expiry ON sessions(expires_at) WHERE revoked_at IS NULL;

CREATE TABLE rooms (
    id          BLOB PRIMARY KEY CHECK (length(id) = 16),
    name        TEXT CHECK (name IS NULL OR length(name) BETWEEN 1 AND 64),
    is_private  INTEGER NOT NULL CHECK (is_private IN (0, 1)),
    owner_id    BLOB REFERENCES users(id),
    dm_a        BLOB REFERENCES users(id) CHECK (dm_a IS NULL OR length(dm_a) = 16),
    dm_b        BLOB REFERENCES users(id) CHECK (dm_b IS NULL OR length(dm_b) = 16),
    created_at  INTEGER NOT NULL CHECK (created_at BETWEEN 1600000000 AND 4000000000),
    mutation_seq INTEGER NOT NULL DEFAULT 0 CHECK (mutation_seq >= 0),
    CHECK ((dm_a IS NULL) = (dm_b IS NULL)),
    CHECK (dm_a IS NULL OR (dm_a < dm_b AND is_private = 1 AND owner_id IS NULL))
) STRICT;

CREATE UNIQUE INDEX dm_pair ON rooms(dm_a, dm_b) WHERE dm_a IS NOT NULL;

CREATE TABLE room_access (
    room_id     BLOB NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    user_id     BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    granted_at  INTEGER NOT NULL CHECK (granted_at BETWEEN 1600000000 AND 4000000000),
    PRIMARY KEY (room_id, user_id)
) STRICT;

CREATE INDEX room_access_user ON room_access(user_id);

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
    body          TEXT CHECK (body IS NULL OR length(body) BETWEEN 1 AND 8000),
    client_nonce  BLOB NOT NULL CHECK (length(client_nonce) = 16),
    created_at    INTEGER NOT NULL CHECK (created_at BETWEEN 1600000000 AND 4000000000),
    edited_at     INTEGER CHECK (edited_at IS NULL OR edited_at >= created_at),
    deleted_at    INTEGER CHECK (deleted_at IS NULL OR deleted_at >= created_at),
    CHECK (deleted_at IS NULL OR body IS NULL)
) STRICT;

CREATE INDEX messages_room ON messages(room_id, seq);
CREATE INDEX messages_room_live ON messages(room_id, seq) WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX message_nonce ON messages(author_id, client_nonce);

CREATE TRIGGER room_access_dm_members BEFORE INSERT ON room_access
WHEN EXISTS (SELECT 1 FROM rooms r WHERE r.id = new.room_id AND r.dm_a IS NOT NULL
                                     AND new.user_id NOT IN (r.dm_a, r.dm_b))
BEGIN SELECT RAISE(ABORT, 'third party in a DM'); END;

CREATE TRIGGER room_access_dm_keep BEFORE DELETE ON room_access
WHEN EXISTS (SELECT 1 FROM rooms r WHERE r.id = old.room_id AND r.dm_a IS NOT NULL)
BEGIN SELECT RAISE(ABORT, 'cannot revoke a DM participant'); END;

-- membership is the read rule for every room, so an author is a member by definition
CREATE TRIGGER messages_author_is_member BEFORE INSERT ON messages
WHEN NOT EXISTS (SELECT 1 FROM room_access a
                  WHERE a.room_id = new.room_id AND a.user_id = new.author_id)
BEGIN SELECT RAISE(ABORT, 'author is not a member of this room'); END;

CREATE TRIGGER messages_author_live BEFORE INSERT ON messages
WHEN (SELECT deleted_at FROM users WHERE id = new.author_id) IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'author is deleted'); END;

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

CREATE TABLE message_attachments (
    message_id  BLOB NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    file_id     BLOB NOT NULL REFERENCES files(id),
    ordinal     INTEGER NOT NULL CHECK (ordinal >= 0 AND ordinal < 32),
    filename    TEXT NOT NULL CHECK (length(filename) BETWEEN 1 AND 255),
    PRIMARY KEY (message_id, ordinal),
    UNIQUE (message_id, file_id)
) STRICT;

CREATE INDEX message_attachments_file ON message_attachments(file_id);

CREATE TABLE user_files (
    user_id   BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    file_id   BLOB NOT NULL REFERENCES files(id),
    filename  TEXT NOT NULL CHECK (length(filename) BETWEEN 1 AND 255),
    added_at  INTEGER NOT NULL CHECK (added_at BETWEEN 1600000000 AND 4000000000),
    PRIMARY KEY (user_id, file_id)
) STRICT;

CREATE INDEX user_files_file ON user_files(file_id);

CREATE TABLE invites (
    code        TEXT PRIMARY KEY
                CHECK (code NOT GLOB '*[^A-Z0-9]*' AND length(code) BETWEEN 12 AND 64),
    created_by  BLOB NOT NULL REFERENCES users(id),
    created_at  INTEGER NOT NULL CHECK (created_at BETWEEN 1600000000 AND 4000000000),
    expires_at  INTEGER CHECK (expires_at IS NULL OR expires_at > created_at),
    max_uses    INTEGER CHECK (max_uses IS NULL OR max_uses > 0),
    uses        INTEGER NOT NULL DEFAULT 0 CHECK (uses >= 0),
    revoked_at  INTEGER CHECK (revoked_at IS NULL OR revoked_at >= created_at),
    CHECK (max_uses IS NULL OR uses <= max_uses)
) STRICT;