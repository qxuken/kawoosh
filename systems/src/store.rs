//! The store (mvp.md Decision 7): SQLite via rusqlite, bundled. Sessions
//! (the layout as JSON), oldfiles, and a namespaced KV table plugins get
//! in one line — `kawoosh.store("myplugin")`. Synchronous on the main
//! thread: every write is a row, and a row is microseconds.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};

pub struct Store {
    conn: Connection,
}

/// `$XDG_DATA_HOME/kawoosh/state.db`, else `~/.local/share/kawoosh/state.db`.
pub fn state_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KAWOOSH_STATE") {
        return Some(PathBuf::from(p));
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("kawoosh/state.db"))
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS kv (
                 ns TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL,
                 PRIMARY KEY (ns, key));
             CREATE TABLE IF NOT EXISTS session (
                 name TEXT PRIMARY KEY, json TEXT NOT NULL, saved_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS oldfiles (
                 path TEXT PRIMARY KEY, opened_at INTEGER NOT NULL, line INTEGER NOT NULL);",
        )?;
        Ok(Self { conn })
    }

    // ---------------------------------------------------------------- kv

    pub fn get(&self, ns: &str, key: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT value FROM kv WHERE ns = ?1 AND key = ?2",
                params![ns, key],
                |r| r.get(0),
            )
            .ok()
    }

    pub fn set(&self, ns: &str, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO kv (ns, key, value) VALUES (?1, ?2, ?3)
             ON CONFLICT (ns, key) DO UPDATE SET value = excluded.value",
            params![ns, key, value],
        )?;
        Ok(())
    }

    pub fn del(&self, ns: &str, key: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM kv WHERE ns = ?1 AND key = ?2",
            params![ns, key],
        )?;
        Ok(())
    }

    pub fn keys(&self, ns: &str) -> Vec<String> {
        let Ok(mut stmt) = self
            .conn
            .prepare("SELECT key FROM kv WHERE ns = ?1 ORDER BY key")
        else {
            return Vec::new();
        };
        stmt.query_map(params![ns], |r| r.get(0))
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }

    // ---------------------------------------------------------------- sessions

    pub fn save_session(&self, name: &str, json: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO session (name, json, saved_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (name) DO UPDATE SET json = excluded.json, saved_at = excluded.saved_at",
            params![name, json, now()],
        )?;
        Ok(())
    }

    pub fn load_session(&self, name: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT json FROM session WHERE name = ?1",
                params![name],
                |r| r.get(0),
            )
            .ok()
    }

    // ---------------------------------------------------------------- oldfiles

    pub fn touch_oldfile(&self, path: &Path, line: usize) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO oldfiles (path, opened_at, line) VALUES (?1, ?2, ?3)
             ON CONFLICT (path) DO UPDATE SET opened_at = excluded.opened_at, line = excluded.line",
            params![path.display().to_string(), now(), line as i64],
        )?;
        Ok(())
    }

    /// Most recent first.
    pub fn oldfiles(&self, limit: usize) -> Vec<(PathBuf, usize)> {
        let Ok(mut stmt) = self
            .conn
            .prepare("SELECT path, line FROM oldfiles ORDER BY opened_at DESC LIMIT ?1")
        else {
            return Vec::new();
        };
        stmt.query_map(params![limit as i64], |r| {
            Ok((
                PathBuf::from(r.get::<_, String>(0)?),
                r.get::<_, i64>(1)? as usize,
            ))
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kv_session_oldfiles() {
        let s = Store::in_memory().unwrap();
        s.set("todo", "k", "v1").unwrap();
        s.set("todo", "k", "v2").unwrap();
        s.set("todo", "a", "x").unwrap();
        assert_eq!(s.get("todo", "k").as_deref(), Some("v2"));
        assert_eq!(s.get("other", "k"), None);
        assert_eq!(s.keys("todo"), ["a", "k"]);
        s.del("todo", "a").unwrap();
        assert_eq!(s.keys("todo"), ["k"]);
        s.save_session("default", "{\"x\":1}").unwrap();
        assert_eq!(s.load_session("default").as_deref(), Some("{\"x\":1}"));
        s.touch_oldfile(Path::new("/a"), 3).unwrap();
        s.touch_oldfile(Path::new("/b"), 7).unwrap();
        let old = s.oldfiles(10);
        assert_eq!(old.len(), 2);
        assert!(old.iter().any(|(p, l)| p == Path::new("/b") && *l == 7));
    }
}
