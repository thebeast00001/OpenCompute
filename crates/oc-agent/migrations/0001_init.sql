-- Milestone 1, first slice: identity, roster and the devices this agent knows.

-- Single values: the cluster this device belongs to, and similar.
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value BLOB NOT NULL
) WITHOUT ROWID;

-- The signed roster log. Entries are only ever added.
CREATE TABLE roster_ops (
    op_id          BLOB PRIMARY KEY,   -- BLAKE3 hash of body
    body           BLOB NOT NULL,      -- the encoded operation, exactly as signed
    signature      BLOB NOT NULL,      -- Ed25519 signature by the cluster root key
    received_at_ms INTEGER NOT NULL
) WITHOUT ROWID;

-- What this agent last learned about each other device.
CREATE TABLE devices (
    device_id       BLOB PRIMARY KEY,
    inventory       BLOB,              -- encoded static inventory
    inventory_at_ms INTEGER,
    last_seen_ms    INTEGER,
    addrs           TEXT NOT NULL DEFAULT ''   -- last known addresses, space separated
) WITHOUT ROWID;
