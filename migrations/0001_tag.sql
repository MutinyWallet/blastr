-- Migration number: 0001 	 2023-08-11T20:22:11.351Z
DROP TABLE IF EXISTS tag;

CREATE TABLE IF NOT EXISTS tag (
    id INTEGER PRIMARY KEY,
    event_id BLOB NOT NULL,
    name TEXT NOT NULL,
    value TEXT,
    FOREIGN KEY(event_id) REFERENCES event(id) ON DELETE CASCADE
    UNIQUE(event_id, name, value)
);

CREATE INDEX IF NOT EXISTS tag_event_index ON tag(event_id);
CREATE INDEX IF NOT EXISTS tag_name_index ON tag(name);
CREATE INDEX IF NOT EXISTS tag_value_index ON tag(value);
