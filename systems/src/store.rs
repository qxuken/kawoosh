//! The store (mvp.md Decision 7): SQLite via rusqlite, bundled. Sessions
//! (the layout as JSON), oldfiles, histories (a buffer's undo tree, and
//! its unsaved text while it has one, kui.md D11), and a namespaced KV
//! table plugins get in one line — `kawoosh.store("myplugin")`.
//! Synchronous on the main thread: every write is a row, and a row is
//! microseconds — a history's is its text, which the app keeps small.
//! WAL, so a write is one transaction that is on disk whole or not at
//! all, and a crash mid-write leaves the row as it was.

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
                 path TEXT PRIMARY KEY, opened_at INTEGER NOT NULL, line INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS histories (
                 key TEXT PRIMARY KEY, text BLOB NOT NULL, meta TEXT NOT NULL,
                 saved_at INTEGER NOT NULL, clean INTEGER NOT NULL DEFAULT 0);",
        )?;
        // A db from before the table's name (`drafts`) or before its
        // `clean` column: moved and added in place; one that is current
        // says so, which is not an error.
        let _ = conn.execute("ALTER TABLE drafts RENAME TO histories", []);
        let _ = conn.execute(
            "ALTER TABLE histories ADD COLUMN clean INTEGER NOT NULL DEFAULT 0",
            [],
        );
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

    // ---------------------------------------------------------------- histories

    /// Keeps `text` and `meta` under `key`, replacing what was there.
    /// `clean`: the row is a saved buffer's history alone, its text on
    /// disk and not here.
    pub fn save_history(
        &self,
        key: &str,
        text: &[u8],
        meta: &str,
        clean: bool,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO histories (key, text, meta, saved_at, clean) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (key) DO UPDATE SET text = excluded.text, meta = excluded.meta,
             saved_at = excluded.saved_at, clean = excluded.clean",
            params![key, text, meta, now(), clean],
        )?;
        Ok(())
    }

    /// The history under `key`: its text and meta.
    pub fn load_history(&self, key: &str) -> Option<(Vec<u8>, String)> {
        self.conn
            .query_row(
                "SELECT text, meta FROM histories WHERE key = ?1",
                params![key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()
    }

    pub fn drop_history(&self, key: &str) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM histories WHERE key = ?1", params![key])?;
        Ok(())
    }

    /// Every history's key, oldest touched first.
    pub fn history_keys(&self) -> Vec<String> {
        self.history_rows().into_iter().map(|r| r.key).collect()
    }

    /// Every history — key, the row's size (text and meta), when it was
    /// last written or claimed, whether it is history alone — oldest
    /// first.
    pub fn history_rows(&self) -> Vec<HistoryRow> {
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT key, length(text) + length(meta), saved_at, clean FROM histories
             ORDER BY saved_at, key",
        ) else {
            return Vec::new();
        };
        stmt.query_map([], |r| {
            Ok(HistoryRow {
                key: r.get(0)?,
                bytes: r.get::<_, i64>(1)? as usize,
                touched_at: r.get(2)?,
                clean: r.get::<_, i64>(3)? != 0,
            })
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
    }

    /// What every row adds up to, text and meta.
    pub fn histories_bytes(&self) -> usize {
        self.conn
            .query_row(
                "SELECT COALESCE(SUM(length(text) + length(meta)), 0) FROM histories",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n as usize)
            .unwrap_or(0)
    }

    /// The history was claimed by a buffer: it counts as touched now, so
    /// a scratch shown every session is never the oldest.
    pub fn touch_history(&self, key: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE histories SET saved_at = ?2 WHERE key = ?1",
            params![key, now()],
        )?;
        Ok(())
    }

    /// Sets when the row was last touched — a tool's, or a test's, way
    /// to age a row.
    pub fn set_history_touched(&self, key: &str, at: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE histories SET saved_at = ?2 WHERE key = ?1",
            params![key, at],
        )?;
        Ok(())
    }

    /// Gives the file's free pages back: after a clear, so the db does
    /// not stay at its high-water mark.
    pub fn vacuum(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch("VACUUM")
    }
}

/// A history as [`Store::history_rows`] lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryRow {
    pub key: String,
    /// Text and meta together.
    pub bytes: usize,
    /// Unix seconds: when the row was last written or claimed.
    pub touched_at: i64,
    /// History alone: the buffer was saved, its text is on disk.
    pub clean: bool,
}

pub fn now() -> i64 {
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

    #[test]
    fn histories_round_trip_and_drop() {
        let s = Store::in_memory().unwrap();
        assert!(s.history_keys().is_empty());
        s.save_history("file:/a", b"one\n", "{}", false).unwrap();
        s.save_history("scratch:1", "два\n".as_bytes(), "{\"n\":1}", false)
            .unwrap();
        s.save_history("file:/a", b"one two\n", "{\"v\":2}", false)
            .unwrap();
        assert_eq!(
            s.load_history("file:/a"),
            Some((b"one two\n".to_vec(), "{\"v\":2}".to_string()))
        );
        assert_eq!(s.history_keys().len(), 2);
        s.drop_history("file:/a").unwrap();
        assert_eq!(s.load_history("file:/a"), None);
        assert_eq!(s.history_keys(), ["scratch:1"]);
        s.drop_history("nothing").unwrap();
        let rows = s.history_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].key.as_str(), rows[0].bytes, rows[0].clean),
            ("scratch:1", 7 + 7, false)
        );
        assert_eq!(s.histories_bytes(), 14);
        assert!(rows[0].touched_at > 0);
        s.save_history("file:/h", b"", "{}", true).unwrap();
        assert!(
            s.history_rows()
                .iter()
                .any(|r| r.key == "file:/h" && r.clean)
        );
        s.set_history_touched("scratch:1", 1).unwrap();
        assert_eq!(s.history_rows()[0].touched_at, 1);
        s.touch_history("scratch:1").unwrap();
        assert!(s.history_rows()[0].touched_at > 1, "claimed: touched now");
        s.vacuum().unwrap();
    }
}
