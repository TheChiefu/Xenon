-- Initial schema.
--
-- Connection settings (foreign_keys, WAL, synchronous, busy_timeout, secure_delete)
-- are NOT set here. They are per-connection concerns and live in SqliteConnectOptions.
--
-- Timestamps are unit-agnostic here: columns carry only relational CHECKs
-- (expires_at > created_at, etc). The unit is decided in Rust by now_ms(),
-- which produces Unix MILLISECONDS. Do not encode a unit in this file.
--
-- Text length CHECKs are the SAME numbers as the product limits in validate.rs.
-- They are not outer bounds. The CHECK enforces the limit on paths that bypass
-- validate.rs: the tombstone path, admin tooling, migrations, the sqlite3 CLI.
-- The two copies cannot be derived from each other, so every bounded column the
-- application writes needs a boundary test at max and max+1. See data-model.md.

CREATE TABLE files (
    id          BLOB PRIMARY KEY CHECK (length(id) = 16),
    sha256      BLOB NOT NULL UNIQUE CHECK (length(sha256) = 32),
    mime        TEXT NOT NULL CHECK (length(mime) BETWEEN 3 AND 255),
    byte_size   INTEGER NOT NULL CHECK (byte_size >= 0),
    created_at  INTEGER NOT NULL
) STRICT;

-- username max is 32 and the tombstone name is a 32-character UUID hex string.
-- There is no headroom. Lowering this breaks account deletion.
CREATE TABLE users (
    id              BLOB PRIMARY KEY CHECK (length(id) = 16),
    username        TEXT NOT NULL UNIQUE
                    CHECK (username NOT GLOB '*[^a-z0-9_-]*' AND length(username) BETWEEN 3 AND 32),
    display_name    TEXT NOT NULL CHECK (length(display_name) BETWEEN 1 AND 64),
    description     TEXT CHECK (description IS NULL OR length(description) <= 2000),
    avatar_file_id  BLOB REFERENCES files(id),
    banner_file_id  BLOB REFERENCES files(id),
    password_hash   TEXT,
    global_role     INTEGER NOT NULL CHECK (global_role IN (0, 1, 2, 3)),
    status_pref     INTEGER NOT NULL DEFAULT 0 CHECK (status_pref IN (0, 1, 2, 3)),
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

-- One room type. A DM is a Hidden room with two members and no permissions;
-- there is no discriminator column and nothing branches on room kind.
--
-- visibility bundles nothing: 0 Public (discoverable, self-join),
-- 1 Locked (discoverable, invite), 2 Hidden (undiscoverable, invite).
-- The undiscoverable-but-self-joinable combination is incoherent and an enum
-- cannot express it. Two booleans could, and would need a CHECK to take it back.
--
-- default_permissions has NO DEFAULT: creation must state what a member with
-- no override may do. 0 is legitimate (read-only room), -1 grants everything.
CREATE TABLE rooms (
    id                  BLOB PRIMARY KEY CHECK (length(id) = 16),
    name                TEXT CHECK (name IS NULL OR length(name) BETWEEN 1 AND 128),
    visibility          INTEGER NOT NULL CHECK (visibility IN (0, 1, 2)),
    default_permissions INTEGER NOT NULL CHECK (default_permissions >= -1),
    created_at          INTEGER NOT NULL,
    mutation_seq        INTEGER NOT NULL DEFAULT 0 CHECK (mutation_seq >= 0)
) STRICT;

-- The directory query. Partial, so Hidden rooms are not in the index at all --
-- smaller, and the index cannot be a route to enumerating them.
-- id is a UUIDv7, so byte order is creation order and it doubles as the cursor.
--
-- Write the query's filter as `visibility IN (0, 1)`, matching this predicate
-- exactly. `visibility != 2` is logically identical but SQLite's partial-index
-- analysis is simple and may not match it. Verify with EXPLAIN QUERY PLAN:
-- expect SEARCH ... USING INDEX rooms_directory, not SCAN rooms.
CREATE INDEX rooms_directory ON rooms(id) WHERE visibility IN (0, 1);

-- Membership, for EVERY room regardless of visibility. This table alone answers
-- "may this user read this room" -- one predicate, no branch.
--
-- permissions NULL means inherit rooms.default_permissions.
-- -1 is the sentinel for all permissions, present and future. A literal
-- "all current bits" would silently withhold every permission added later.
--
-- granted_at is load-bearing, not display-only: it is the promotion sort key
-- when a Public room loses its last delete-room holder.
CREATE TABLE room_access (
    room_id     BLOB NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    user_id     BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    permissions INTEGER CHECK (permissions IS NULL OR permissions >= -1),
    granted_at  INTEGER NOT NULL,
    PRIMARY KEY (room_id, user_id)
) STRICT;

CREATE INDEX room_access_user ON room_access(user_id);

-- Pending invitations to Locked and Hidden rooms. A row means invited, not joined.
-- Deliberately a separate table rather than a nullable accepted_at on room_access:
-- that column would have to be filtered by read authorization, the author-is-member
-- trigger, the zero-member count and the promotion query, and forgetting one is
-- silent over-inclusion. A missed check here just means invites do not work.
CREATE TABLE room_invites (
    room_id     BLOB NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    user_id     BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    invited_by  BLOB REFERENCES users(id),
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER CHECK (expires_at IS NULL OR expires_at > created_at),
    PRIMARY KEY (room_id, user_id)
) STRICT;

CREATE INDEX room_invites_user ON room_invites(user_id);

-- Room-scoped. Banning also removes the room_access row, so a banned user is not
-- a member and nothing in the permission path consults this table -- it is read
-- at exactly one place, the join.
--
-- Expired rows are NOT swept: the join check must test expires_at regardless,
-- so a sweep would reclaim storage and lose the moderation record.
--
-- created_by is plain REFERENCES, never CASCADE. If it cascaded, hard-deleting a
-- moderator would silently lift every ban they issued.
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
    body          TEXT CHECK (body IS NULL OR length(body) BETWEEN 1 AND 8000),
    client_nonce  BLOB NOT NULL CHECK (length(client_nonce) = 16),
    created_at    INTEGER NOT NULL,
    edited_at     INTEGER CHECK (edited_at IS NULL OR edited_at >= created_at),
    deleted_at    INTEGER CHECK (deleted_at IS NULL OR deleted_at >= created_at),
    CHECK (deleted_at IS NULL OR body IS NULL)
) STRICT;

CREATE INDEX messages_room ON messages(room_id, seq);
CREATE INDEX messages_room_live ON messages(room_id, seq) WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX message_nonce ON messages(author_id, client_nonce);

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
    added_at  INTEGER NOT NULL,
    PRIMARY KEY (user_id, file_id)
) STRICT;

CREATE INDEX user_files_file ON user_files(file_id);

-- Server registration invites. Distinct from room_invites, which govern entry to
-- a room by someone who already has an account.
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