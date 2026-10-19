//! SQLite persistence for the URL frontier.
//!
//! The frontier remains an in-memory priority queue during a crawl; this
//! module records queued URLs, the seen set, and in-flight leases so a
//! process restart resumes instead of re-fetching everything.

use std::collections::HashSet;
use std::path::Path;

use rusqlite::{params, Connection};
use url::Url;

use crate::item::FrontierItem;

pub struct FrontierStore {
    conn: Connection,
}

pub struct LoadedItem {
    pub url: Url,
    pub depth: usize,
    pub discovered_from: Option<Url>,
    pub priority: i64,
    pub seq: u64,
}

impl FrontierStore {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS seen (
                fp INTEGER PRIMARY KEY
            );
            CREATE TABLE IF NOT EXISTS items (
                fp INTEGER PRIMARY KEY,
                url TEXT NOT NULL,
                depth INTEGER NOT NULL,
                discovered_from TEXT,
                priority INTEGER NOT NULL,
                seq INTEGER NOT NULL,
                state TEXT NOT NULL DEFAULT 'queued'
            );
            CREATE INDEX IF NOT EXISTS idx_items_state_priority
                ON items(state, priority DESC, seq ASC);
            "#,
        )?;
        Ok(Self { conn })
    }

    /// Requeue any leases left in-flight by a crash, then load queued work and
    /// the seen set. Returns items in priority/FIFO order.
    pub fn load(&self) -> rusqlite::Result<(HashSet<u64>, Vec<LoadedItem>, u64)> {
        self.conn.execute(
            "UPDATE items SET state = 'queued' WHERE state = 'inflight'",
            [],
        )?;
        let mut seen = HashSet::new();
        {
            let mut stmt = self.conn.prepare("SELECT fp FROM seen")?;
            let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
            for row in rows {
                seen.insert(row? as u64);
            }
        }
        let mut items = Vec::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT url, depth, discovered_from, priority, seq FROM items \
                 WHERE state = 'queued' ORDER BY priority DESC, seq ASC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?;
            for row in rows {
                let (url, depth, discovered, priority, seq) = row?;
                let Ok(url) = Url::parse(&url) else { continue };
                let discovered = discovered.and_then(|value| Url::parse(&value).ok());
                items.push(LoadedItem {
                    url,
                    depth: depth as usize,
                    discovered_from: discovered,
                    priority,
                    seq: seq as u64,
                });
            }
        }
        let max_seq = items.iter().map(|item| item.seq).max().unwrap_or(0);
        Ok((seen, items, max_seq))
    }

    pub fn mark_seen(&self, fingerprint: u64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO seen (fp) VALUES (?1)",
            params![fingerprint as i64],
        )?;
        Ok(())
    }

    pub fn enqueue(&self, fingerprint: u64, item: &FrontierItem) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO items \
             (fp, url, depth, discovered_from, priority, seq, state) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'queued')",
            params![
                fingerprint as i64,
                item.url.as_str(),
                item.depth as i64,
                item.discovered_from
                    .as_ref()
                    .map(|url| url.as_str().to_string()),
                item.priority,
                item.seq as i64,
            ],
        )?;
        Ok(())
    }

    pub fn mark_inflight(&self, fingerprint: u64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE items SET state = 'inflight' WHERE fp = ?1",
            params![fingerprint as i64],
        )?;
        Ok(())
    }

    pub fn complete(&self, fingerprint: u64) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM items WHERE fp = ?1",
            params![fingerprint as i64],
        )?;
        Ok(())
    }
}
