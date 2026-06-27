use rusqlite::{Connection, params};

use crate::config::AgentConfig;
use crate::error::Result;

/// Persists the last-known-good config locally so the agent can recover
/// after receiving a bad config from the control plane.
pub struct ConfigStore {
    conn: Connection,
}

impl ConfigStore {
    pub fn open(db_path: &str) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS config (
                id      INTEGER PRIMARY KEY CHECK (id = 1),
                payload TEXT NOT NULL
            );",
        )?;
        Ok(Self { conn })
    }

    pub fn save(&self, cfg: &AgentConfig) -> Result<()> {
        let json = serde_json::to_string(cfg)?;
        self.conn.execute(
            "INSERT INTO config (id, payload) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET payload = excluded.payload",
            params![json],
        )?;
        Ok(())
    }

    pub fn load(&self) -> Result<Option<AgentConfig>> {
        let mut stmt = self
            .conn
            .prepare("SELECT payload FROM config WHERE id = 1")?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let json: String = row.get(0)?;
            let cfg: AgentConfig = serde_json::from_str(&json)?;
            return Ok(Some(cfg));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_config() {
        let store = ConfigStore::open(":memory:").unwrap();
        let cfg = AgentConfig::default();
        store.save(&cfg).unwrap();
        let loaded = store.load().unwrap().expect("config should exist");
        assert_eq!(cfg, loaded);
    }
}
