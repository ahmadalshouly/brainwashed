CREATE TABLE IF NOT EXISTS addresses (
  id TEXT PRIMARY KEY,
  url TEXT NOT NULL,
  updated INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS addresses_updated ON addresses (updated);
