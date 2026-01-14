use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use rusqlite_migration::{Migrations, M};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcription {
    pub id: String,
    pub timestamp: i64,
    pub text: String,
    pub source_node: String,
    pub memo_device_id: Option<String>,
    pub synced: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peer {
    pub node_id: String,
    pub last_seen: i64,
    pub last_sync_timestamp: i64,
}

#[derive(Clone)]
pub struct Storage {
    conn: Arc<Mutex<Connection>>,
}

impl Storage {
    pub fn new(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)
            .with_context(|| format!("Failed to open database at {}", path.display()))?;

        let migrations = Migrations::new(vec![
            M::up(
                "CREATE TABLE transcriptions (
                    id TEXT PRIMARY KEY,
                    timestamp INTEGER NOT NULL,
                    text TEXT NOT NULL,
                    source_node TEXT NOT NULL,
                    memo_device_id TEXT,
                    synced INTEGER DEFAULT 0
                );

                CREATE INDEX idx_timestamp ON transcriptions(timestamp);
                CREATE INDEX idx_synced ON transcriptions(synced);

                CREATE TABLE peers (
                    node_id TEXT PRIMARY KEY,
                    last_seen INTEGER,
                    last_sync_timestamp INTEGER
                );",
            ),
        ]);

        migrations
            .to_latest(&mut conn)
            .context("Failed to run migrations")?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn insert_transcription(&self, transcription: &Transcription) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO transcriptions (id, timestamp, text, source_node, memo_device_id, synced)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                transcription.id,
                transcription.timestamp,
                transcription.text,
                transcription.source_node,
                transcription.memo_device_id,
                transcription.synced as i32,
            ],
        )
        .context("Failed to insert transcription")?;
        Ok(())
    }

    pub fn get_transcriptions_since(&self, since: i64) -> Result<Vec<Transcription>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, timestamp, text, source_node, memo_device_id, synced FROM transcriptions WHERE timestamp > ?1 ORDER BY timestamp ASC")
            .context("Failed to prepare statement")?;

        let transcriptions = stmt
            .query_map(params![since], |row| {
                Ok(Transcription {
                    id: row.get(0)?,
                    timestamp: row.get(1)?,
                    text: row.get(2)?,
                    source_node: row.get(3)?,
                    memo_device_id: row.get(4)?,
                    synced: row.get::<_, i32>(5)? != 0,
                })
            })
            .context("Failed to query transcriptions")?
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to collect transcriptions")?;

        Ok(transcriptions)
    }

    pub fn get_recent_transcriptions(&self, limit: usize) -> Result<Vec<Transcription>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, timestamp, text, source_node, memo_device_id, synced FROM transcriptions ORDER BY timestamp DESC LIMIT ?1")
            .context("Failed to prepare statement")?;

        let transcriptions = stmt
            .query_map(params![limit], |row| {
                Ok(Transcription {
                    id: row.get(0)?,
                    timestamp: row.get(1)?,
                    text: row.get(2)?,
                    source_node: row.get(3)?,
                    memo_device_id: row.get(4)?,
                    synced: row.get::<_, i32>(5)? != 0,
                })
            })
            .context("Failed to query transcriptions")?
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to collect transcriptions")?;

        Ok(transcriptions)
    }

    pub fn count_transcriptions(&self) -> Result<(usize, usize)> {
        let conn = self.conn.lock().unwrap();
        let total: usize = conn
            .query_row("SELECT COUNT(*) FROM transcriptions", [], |row| row.get(0))
            .context("Failed to count total transcriptions")?;
        let synced: usize = conn
            .query_row(
                "SELECT COUNT(*) FROM transcriptions WHERE synced = 1",
                [],
                |row| row.get(0),
            )
            .context("Failed to count synced transcriptions")?;
        Ok((total, synced))
    }

    pub fn mark_synced(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE transcriptions SET synced = 1 WHERE id = ?1", params![id])
            .context("Failed to mark transcription as synced")?;
        Ok(())
    }

    pub fn upsert_peer(&self, peer: &Peer) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO peers (node_id, last_seen, last_sync_timestamp)
             VALUES (?1, ?2, ?3)",
            params![peer.node_id, peer.last_seen, peer.last_sync_timestamp],
        )
        .context("Failed to upsert peer")?;
        Ok(())
    }

    pub fn get_peers(&self) -> Result<Vec<Peer>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT node_id, last_seen, last_sync_timestamp FROM peers")
            .context("Failed to prepare statement")?;

        let peers = stmt
            .query_map([], |row| {
                Ok(Peer {
                    node_id: row.get(0)?,
                    last_seen: row.get(1)?,
                    last_sync_timestamp: row.get(2)?,
                })
            })
            .context("Failed to query peers")?
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to collect peers")?;

        Ok(peers)
    }

    pub fn get_peer(&self, node_id: &str) -> Result<Option<Peer>> {
        let conn = self.conn.lock().unwrap();
        let peer = conn
            .query_row(
                "SELECT node_id, last_seen, last_sync_timestamp FROM peers WHERE node_id = ?1",
                params![node_id],
                |row| {
                    Ok(Peer {
                        node_id: row.get(0)?,
                        last_seen: row.get(1)?,
                        last_sync_timestamp: row.get(2)?,
                    })
                },
            )
            .optional()
            .context("Failed to query peer")?;

        Ok(peer)
    }
}
