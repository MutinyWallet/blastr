-- Migration number: 0000 	 2023-08-10T16:43:44.275Z
DROP TABLE IF EXISTS event;

CREATE TABLE IF NOT EXISTS event (
    id BLOB PRIMARY KEY,
    created_at INTEGER NOT NULL,
    pubkey BLOB NOT NULL,
    kind INTEGER NOT NULL,
    content TEXT NOT NULL,
    sig BLOB NOT NULL,
    deleted INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS pubkey_index ON event(pubkey);
CREATE INDEX IF NOT EXISTS kind_index ON event(kind);
