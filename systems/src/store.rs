//! The store (mvp.md Decision 7): SQLite via rusqlite, bundled. Sessions
//! (the layout as JSON), the memory (memory.md: a `moments` row per
//! subject attended and a `recent` ring of the transitions between
//! them), histories (a buffer's undo tree, and its unsaved text while
//! it has one, kui.md D11), and a namespaced KV table plugins get in
//! one line — `kawoosh.store("myplugin")`. Synchronous on the main
//! thread: every write is a row, and a row is microseconds — a
//! history's is its text, which the app keeps small. WAL, so a write
//! is one transaction that is on disk whole or not at all, and a crash
//! mid-write leaves the row as it was. Two windows on one db: a
//! moment's counters are written as increments (`flush_moments`), so
//! neither window loses the other's, and a writer that meets the lock
//! waits [`BUSY_MS`] and then gets `SQLITE_BUSY` back to try again
//! later with what it still holds.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};

/// How long a write waits for another connection's lock before it
/// gives up with `SQLITE_BUSY` (memory.md Decision 3).
pub const BUSY_MS: u64 = 250;

pub struct Store {
    conn: Connection,
}

/// A moment's identity: its kind, its subject and the workspace it was
/// made under (memory.md Decision 1).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MomentKey {
    pub kind: String,
    pub subject: String,
    pub workspace: String,
}

impl MomentKey {
    pub fn new(kind: &str, subject: &str, workspace: &str) -> Self {
        Self {
            kind: kind.into(),
            subject: subject.into(),
            workspace: workspace.into(),
        }
    }
}

/// A moment's row as the store lists it: its header — never a text's
/// bytes, only their count and first line (memory.md Decision 3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MomentRow {
    pub key: MomentKey,
    pub first_at: i64,
    pub last_at: i64,
    pub visits: i64,
    pub dwell_ms: i64,
    pub edits: i64,
    pub yanks: i64,
    /// 0 for not pinned; else the pin's ordinal (Decision 5).
    pub pinned: i64,
    /// JSON: what the kind carries besides.
    pub meta: String,
    /// A text moment's bytes, counted.
    pub text_len: Option<usize>,
    /// A text moment's first line, up to [`TEXT_HEAD`] bytes.
    pub text_head: Option<String>,
}

/// How much of a text's first line a row carries.
pub const TEXT_HEAD: usize = 200;

/// What happened to a subject since the last flush: counters to add,
/// and what to set (memory.md Decision 3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MomentDelta {
    pub visits: i64,
    pub dwell_ms: i64,
    pub edits: i64,
    pub yanks: i64,
    /// When the subject was first and last attended in this delta.
    pub first_at: i64,
    pub last_at: i64,
    /// Set when given; the row keeps its own otherwise.
    pub meta: Option<String>,
    /// A text moment's bytes, written once.
    pub text: Option<Vec<u8>>,
}

/// One row of the ring: a visit, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RingRow {
    pub at: i64,
    pub key: MomentKey,
}

/// What the memory holds that the store does not yet (memory.md
/// Decision 3): a delta per subject since the last flush, and the ring
/// rows pending, in order. Shared between the shell's `moments.rs`,
/// which writes it, and the Lua runtime, which folds it into what
/// `kawoosh.memory { … }` answers — so a file opened a moment ago has
/// its row before the flush.
#[derive(Clone, Debug, Default)]
pub struct PendingMoments {
    pub deltas: std::collections::HashMap<MomentKey, MomentDelta>,
    pub ring: Vec<RingRow>,
}

/// A query over the moments (`kawoosh.memory { … }`, the pane).
#[derive(Clone, Debug, Default)]
pub struct MomentQuery<'a> {
    pub kind: Option<&'a str>,
    /// `Some("")` for moments made outside any workspace; `None` for
    /// every workspace.
    pub workspace: Option<&'a str>,
    pub subject: Option<&'a str>,
    /// Rows attended since this time.
    pub since: Option<i64>,
    pub pinned: bool,
    pub limit: usize,
}

/// `$XDG_DATA_HOME/kawoosh/state.db`, else `~/.local/share/kawoosh/state.db`.
pub fn state_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KAWOOSH_STATE") {
        return Some(PathBuf::from(p));
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| crate::fs::home().map(|h| h.join(".local").join("share")))?;
    Some(base.join("kawoosh").join("state.db"))
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
        conn.busy_timeout(std::time::Duration::from_millis(BUSY_MS))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS kv (
                 ns TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL,
                 PRIMARY KEY (ns, key));
             CREATE TABLE IF NOT EXISTS session (
                 name TEXT PRIMARY KEY, json TEXT NOT NULL, saved_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS histories (
                 key TEXT PRIMARY KEY, text BLOB NOT NULL, meta TEXT NOT NULL,
                 saved_at INTEGER NOT NULL, clean INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS moments (
                 kind      TEXT    NOT NULL,
                 subject   TEXT    NOT NULL,
                 workspace TEXT    NOT NULL DEFAULT '',
                 first_at  INTEGER NOT NULL,
                 last_at   INTEGER NOT NULL,
                 visits    INTEGER NOT NULL DEFAULT 0,
                 dwell_ms  INTEGER NOT NULL DEFAULT 0,
                 edits     INTEGER NOT NULL DEFAULT 0,
                 yanks     INTEGER NOT NULL DEFAULT 0,
                 pinned    INTEGER NOT NULL DEFAULT 0,
                 meta      TEXT    NOT NULL DEFAULT '{}',
                 text      BLOB,
                 PRIMARY KEY (kind, subject, workspace));
             CREATE INDEX IF NOT EXISTS moments_last ON moments (kind, last_at);
             CREATE TABLE IF NOT EXISTS recent (
                 at        INTEGER NOT NULL,
                 kind      TEXT    NOT NULL,
                 subject   TEXT    NOT NULL,
                 workspace TEXT    NOT NULL DEFAULT '');
             CREATE INDEX IF NOT EXISTS recent_at ON recent (at);",
        )?;
        // A db from before the table's name (`drafts`) or before its
        // `clean` column: moved and added in place; one that is current
        // says so, which is not an error.
        let _ = conn.execute("ALTER TABLE drafts RENAME TO histories", []);
        let _ = conn.execute(
            "ALTER TABLE histories ADD COLUMN clean INTEGER NOT NULL DEFAULT 0",
            [],
        );
        // A db from before the memory (memory.md Decisions 2 and 6):
        // its `oldfiles` become `file` moments, one visit each at the
        // time they were opened with the line they were left at, and
        // the table goes; a history without a moment gets one, last
        // attended when its row was written. Both idempotent, so a db
        // that is current is left as it is — a history's moment may
        // live under a workspace (the shell keys a file by the root it
        // was opened under), so one under *any* workspace counts, else
        // every launch would twin it with an empty row under none.
        let _ = conn.execute(
            "INSERT OR IGNORE INTO moments (kind, subject, workspace, first_at, last_at, visits, meta)
             SELECT 'file', path, '', opened_at, opened_at, 1, json_object('line', line)
             FROM oldfiles",
            [],
        );
        let _ = conn.execute("DROP TABLE IF EXISTS oldfiles", []);
        conn.execute(
            "INSERT INTO moments (kind, subject, workspace, first_at, last_at, visits)
             SELECT h.kind, h.subject, '', h.saved_at, h.saved_at, 0
             FROM (SELECT CASE WHEN key LIKE 'file:%' THEN 'file' ELSE 'scratch' END AS kind,
                          CASE WHEN key LIKE 'file:%' THEN substr(key, 6) ELSE key END AS subject,
                          saved_at
                   FROM histories
                   WHERE key LIKE 'file:%' OR key LIKE 'scratch:%') AS h
             WHERE NOT EXISTS (SELECT 1 FROM moments m
                               WHERE m.kind = h.kind AND m.subject = h.subject)",
            [],
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

    // ---------------------------------------------------------------- moments

    /// Every delta as one upsert whose counters are increments and
    /// whose `meta` is merged over the row's (SQLite's `json_patch`, so
    /// the engine's caret line and a plugin's keys keep out of each
    /// other's way), the ring's new rows after them and the ring
    /// trimmed to `ring_max`, in one transaction: whole or nothing. A
    /// `SQLITE_BUSY` (another window held the lock past [`BUSY_MS`])
    /// comes back as the error, and the caller keeps its deltas for
    /// the next tick. Answers how many rows the flush made — the
    /// deltas whose key the table had not — since only a row made can
    /// bring a kind past its cap (memory.md Decision 4).
    pub fn flush_moments(
        &self,
        deltas: &[(MomentKey, MomentDelta)],
        ring: &[RingRow],
        ring_max: usize,
    ) -> rusqlite::Result<usize> {
        // The write lock first (`BEGIN IMMEDIATE`, waited for up to
        // [`BUSY_MS`]): a count read before it would open a read
        // transaction, and SQLite refuses to raise one to a write
        // while another window writes, at once and without waiting.
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let made = self.flush_moments_in(deltas, ring, ring_max);
        match made {
            Ok(made) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(made)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// [`Self::flush_moments`] inside its transaction.
    fn flush_moments_in(
        &self,
        deltas: &[(MomentKey, MomentDelta)],
        ring: &[RingRow],
        ring_max: usize,
    ) -> rusqlite::Result<usize> {
        let rows = || -> rusqlite::Result<i64> {
            self.conn
                .query_row("SELECT count(*) FROM moments", [], |r| r.get(0))
        };
        let before = rows()?;
        {
            let mut up = self.conn.prepare_cached(
                "INSERT INTO moments (kind, subject, workspace, first_at, last_at,
                                      visits, dwell_ms, edits, yanks, meta, text)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, coalesce(?10, '{}'), ?11)
                 ON CONFLICT (kind, subject, workspace) DO UPDATE SET
                     first_at = min(first_at, excluded.first_at),
                     last_at  = max(last_at, excluded.last_at),
                     visits   = visits + excluded.visits,
                     dwell_ms = dwell_ms + excluded.dwell_ms,
                     edits    = edits + excluded.edits,
                     yanks    = yanks + excluded.yanks,
                     meta     = json_patch(meta, coalesce(?10, '{}')),
                     text     = coalesce(excluded.text, text)",
            )?;
            for (k, d) in deltas {
                up.execute(params![
                    k.kind,
                    k.subject,
                    k.workspace,
                    d.first_at,
                    d.last_at,
                    d.visits,
                    d.dwell_ms,
                    d.edits,
                    d.yanks,
                    d.meta,
                    d.text,
                ])?;
            }
            let mut ins = self.conn.prepare_cached(
                "INSERT INTO recent (at, kind, subject, workspace) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for r in ring {
                ins.execute(params![r.at, r.key.kind, r.key.subject, r.key.workspace])?;
            }
        }
        if !ring.is_empty() {
            self.conn.execute(
                "DELETE FROM recent WHERE rowid NOT IN
                     (SELECT rowid FROM recent ORDER BY at DESC, rowid DESC LIMIT ?1)",
                params![ring_max as i64],
            )?;
        }
        Ok((rows()? - before).max(0) as usize)
    }

    /// The rows a query names, last attended first.
    pub fn moments(&self, q: &MomentQuery<'_>) -> Vec<MomentRow> {
        let mut sql = String::from(
            "SELECT kind, subject, workspace, first_at, last_at, visits, dwell_ms, edits, yanks,
                    pinned, meta, length(text), substr(text, 1, ?1)
             FROM moments WHERE 1 = 1",
        );
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(TEXT_HEAD as i64)];
        if let Some(k) = q.kind {
            sql.push_str(" AND kind = ?");
            args.push(Box::new(k.to_string()));
            sql.push_str(&args.len().to_string());
        }
        if let Some(w) = q.workspace {
            sql.push_str(" AND workspace = ?");
            args.push(Box::new(w.to_string()));
            sql.push_str(&args.len().to_string());
        }
        if let Some(s) = q.subject {
            sql.push_str(" AND subject = ?");
            args.push(Box::new(s.to_string()));
            sql.push_str(&args.len().to_string());
        }
        if let Some(t) = q.since {
            sql.push_str(" AND last_at >= ?");
            args.push(Box::new(t));
            sql.push_str(&args.len().to_string());
        }
        if q.pinned {
            sql.push_str(" AND pinned > 0 ORDER BY pinned");
        } else {
            sql.push_str(" ORDER BY last_at DESC, rowid DESC");
        }
        if q.limit > 0 {
            sql.push_str(" LIMIT ?");
            args.push(Box::new(q.limit as i64));
            sql.push_str(&args.len().to_string());
        }
        let Ok(mut stmt) = self.conn.prepare(&sql) else {
            return Vec::new();
        };
        let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a.as_ref()).collect();
        stmt.query_map(refs.as_slice(), row_of)
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }

    /// One row.
    pub fn moment(&self, key: &MomentKey) -> Option<MomentRow> {
        self.conn
            .query_row(
                "SELECT kind, subject, workspace, first_at, last_at, visits, dwell_ms, edits, yanks,
                        pinned, meta, length(text), substr(text, 1, ?4)
                 FROM moments WHERE kind = ?1 AND subject = ?2 AND workspace = ?3",
                params![key.kind, key.subject, key.workspace, TEXT_HEAD as i64],
                row_of,
            )
            .ok()
    }

    /// A text moment's bytes, read when they are wanted.
    pub fn moment_text(&self, key: &MomentKey) -> Option<Vec<u8>> {
        self.conn
            .query_row(
                "SELECT text FROM moments WHERE kind = ?1 AND subject = ?2 AND workspace = ?3",
                params![key.kind, key.subject, key.workspace],
                |r| r.get::<_, Option<Vec<u8>>>(0),
            )
            .ok()
            .flatten()
    }

    /// Sets when a row was last attended — a tool's, or a test's, way
    /// to age it.
    pub fn set_moment_last(&self, key: &MomentKey, at: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE moments SET last_at = ?4 WHERE kind = ?1 AND subject = ?2 AND workspace = ?3",
            params![key.kind, key.subject, key.workspace, at],
        )?;
        Ok(())
    }

    /// The user's word on a row: its pin ordinal, 0 for none.
    pub fn set_pinned(&self, key: &MomentKey, ordinal: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE moments SET pinned = ?4 WHERE kind = ?1 AND subject = ?2 AND workspace = ?3",
            params![key.kind, key.subject, key.workspace, ordinal],
        )?;
        Ok(())
    }

    /// The row and its ring rows gone. The history under a `file` or
    /// `scratch` subject goes with it (memory.md Decision 6) — once no
    /// row of the same subject under another workspace still owns it:
    /// a history is the path's, a moment the path's under a root.
    pub fn forget_moment(&self, key: &MomentKey) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM moments WHERE kind = ?1 AND subject = ?2 AND workspace = ?3",
            params![key.kind, key.subject, key.workspace],
        )?;
        tx.execute(
            "DELETE FROM recent WHERE kind = ?1 AND subject = ?2 AND workspace = ?3",
            params![key.kind, key.subject, key.workspace],
        )?;
        if let Some(hk) = history_key_of(key) {
            tx.execute(
                "DELETE FROM histories WHERE key = ?1
                 AND NOT EXISTS (SELECT 1 FROM moments WHERE kind = ?2 AND subject = ?3)",
                params![hk, key.kind, key.subject],
            )?;
        }
        tx.commit()
    }

    /// The ring, newest first.
    pub fn recent(&self, limit: usize) -> Vec<RingRow> {
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT at, kind, subject, workspace FROM recent
             ORDER BY at DESC, rowid DESC LIMIT ?1",
        ) else {
            return Vec::new();
        };
        stmt.query_map(params![limit as i64], |r| {
            Ok(RingRow {
                at: r.get(0)?,
                key: MomentKey {
                    kind: r.get(1)?,
                    subject: r.get(2)?,
                    workspace: r.get(3)?,
                },
            })
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
    }

    /// How many rows the ring holds.
    pub fn recent_len(&self) -> usize {
        self.conn
            .query_row("SELECT count(*) FROM recent", [], |r| r.get::<_, i64>(0))
            .map(|n| n as usize)
            .unwrap_or(0)
    }

    /// What the moments weigh: their texts, metas and subjects.
    pub fn moments_bytes(&self) -> usize {
        self.conn
            .query_row(
                "SELECT COALESCE(SUM(COALESCE(length(text), 0) + length(meta) + length(subject)), 0)
                 FROM moments",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n as usize)
            .unwrap_or(0)
    }

    /// How many rows each kind has: the per-kind caps read off the
    /// index, without the rows.
    pub fn kind_counts(&self) -> Vec<(String, usize)> {
        let Ok(mut stmt) = self
            .conn
            .prepare("SELECT kind, count(*) FROM moments GROUP BY kind")
        else {
            return Vec::new();
        };
        stmt.query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as usize)))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    /// What the text moments' bytes add up to.
    pub fn text_bytes(&self) -> usize {
        self.conn
            .query_row(
                "SELECT COALESCE(SUM(length(text)), 0) FROM moments WHERE kind = 'text'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n as usize)
            .unwrap_or(0)
    }

    /// Every moment, ring row and history gone (`:memory clear`).
    pub fn clear_moments(&self) -> rusqlite::Result<()> {
        self.conn
            .execute_batch("DELETE FROM moments; DELETE FROM recent; DELETE FROM histories;")
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

    /// Sets when the row was written — a tool's, or a test's, way to
    /// age a row. (A history's age for keeping is its moment's
    /// `last_at`, memory.md Decision 6; this is the row's own stamp.)
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

/// `rows` (a query's answer) with the pending deltas folded in for the
/// subjects `q` names — so a file opened a moment ago is listed before
/// the flush, by the pane and by `kawoosh.memory` alike. The order and
/// the limit are re-applied after.
pub fn fold_pending(rows: &mut Vec<MomentRow>, pending: &PendingMoments, q: &MomentQuery<'_>) {
    for (k, d) in &pending.deltas {
        if q.kind.is_some_and(|kind| kind != k.kind)
            || q.workspace.is_some_and(|w| w != k.workspace)
            || q.subject.is_some_and(|s| s != k.subject)
            || q.pinned
        {
            continue;
        }
        match rows.iter_mut().find(|r| r.key == *k) {
            Some(r) => {
                r.visits += d.visits;
                r.edits += d.edits;
                r.yanks += d.yanks;
                r.dwell_ms += d.dwell_ms;
                r.last_at = r.last_at.max(d.last_at);
                if let Some(m) = &d.meta {
                    r.meta = merge_meta(&r.meta, m);
                }
            }
            None => {
                if q.since.is_some_and(|t| d.last_at < t) {
                    continue;
                }
                rows.push(MomentRow {
                    key: k.clone(),
                    first_at: d.first_at,
                    last_at: d.last_at,
                    visits: d.visits,
                    dwell_ms: d.dwell_ms,
                    edits: d.edits,
                    yanks: d.yanks,
                    pinned: 0,
                    meta: d.meta.clone().unwrap_or_else(|| "{}".into()),
                    text_len: d.text.as_ref().map(Vec::len),
                    text_head: d.text.as_ref().map(|t| {
                        String::from_utf8_lossy(t)
                            .lines()
                            .next()
                            .unwrap_or("")
                            .to_string()
                    }),
                });
            }
        }
    }
    if !q.pinned {
        rows.sort_by_key(|r| std::cmp::Reverse(r.last_at));
    }
    if q.limit > 0 {
        rows.truncate(q.limit);
    }
}

/// `patch`'s keys written over `meta`'s (what `json_patch` does in the
/// flush), both JSON objects; a side that is not one is taken whole.
pub fn merge_meta(meta: &str, patch: &str) -> String {
    let base: serde_json::Value = serde_json::from_str(meta).unwrap_or_default();
    let over: serde_json::Value = serde_json::from_str(patch).unwrap_or_default();
    match (base, over) {
        (serde_json::Value::Object(mut b), serde_json::Value::Object(o)) => {
            b.extend(o);
            serde_json::Value::Object(b).to_string()
        }
        (_, over) if over.is_object() => over.to_string(),
        (base, _) => base.to_string(),
    }
}

fn row_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<MomentRow> {
    let text_len = r.get::<_, Option<i64>>(11)?.map(|n| n as usize);
    let head = r.get::<_, Option<Vec<u8>>>(12)?;
    Ok(MomentRow {
        key: MomentKey {
            kind: r.get(0)?,
            subject: r.get(1)?,
            workspace: r.get(2)?,
        },
        first_at: r.get(3)?,
        last_at: r.get(4)?,
        visits: r.get(5)?,
        dwell_ms: r.get(6)?,
        edits: r.get(7)?,
        yanks: r.get(8)?,
        pinned: r.get(9)?,
        meta: r.get(10)?,
        text_len,
        text_head: head.map(|h| {
            let s = String::from_utf8_lossy(&h);
            s.lines().next().unwrap_or("").to_string()
        }),
    })
}

/// A file row's `meta.line` (the caret line it was left at), 0 when
/// it has none.
pub fn meta_line(meta: &str) -> usize {
    serde_json::from_str::<serde_json::Value>(meta)
        .ok()
        .and_then(|v| v.get("line")?.as_u64())
        .unwrap_or(0) as usize
}

/// The history row a `file` or `scratch` moment owns (memory.md
/// Decision 6): `file:<path>`, or the scratch's own subject.
pub fn history_key_of(key: &MomentKey) -> Option<String> {
    match key.kind.as_str() {
        "file" => Some(format!("file:{}", key.subject)),
        "scratch" => Some(key.subject.clone()),
        _ => None,
    }
}

/// The moment a history row belongs to.
pub fn moment_key_of_history(key: &str) -> Option<MomentKey> {
    if let Some(p) = key.strip_prefix("file:") {
        Some(MomentKey::new("file", p, ""))
    } else if key.starts_with("scratch:") {
        Some(MomentKey::new("scratch", key, ""))
    } else {
        None
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
    }

    fn visit(kind: &str, subject: &str, at: i64) -> (MomentKey, MomentDelta) {
        (
            MomentKey::new(kind, subject, ""),
            MomentDelta {
                visits: 1,
                first_at: at,
                last_at: at,
                ..Default::default()
            },
        )
    }

    /// A flush upserts by increments, so two connections' halves add
    /// up; the ring keeps its cap; a query reads headers, never a
    /// text's bytes; forgetting takes the ring rows and the history.
    #[test]
    fn moments_add_up_and_the_ring_is_capped() {
        let dir = std::env::temp_dir().join(format!("kawoosh-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("m.db");
        let _ = std::fs::remove_file(&db);
        let a = Store::open(&db).unwrap();
        let b = Store::open(&db).unwrap();
        let ring = |at: i64, subject: &str| RingRow {
            at,
            key: MomentKey::new("file", subject, ""),
        };
        a.flush_moments(&[visit("file", "/x", 10)], &[ring(10, "/x")], 3)
            .unwrap();
        b.flush_moments(
            &[
                visit("file", "/x", 12),
                (
                    MomentKey::new("file", "/y", ""),
                    MomentDelta {
                        edits: 2,
                        dwell_ms: 500,
                        first_at: 11,
                        last_at: 11,
                        meta: Some("{\"line\":4}".into()),
                        ..Default::default()
                    },
                ),
            ],
            &[ring(11, "/y"), ring(12, "/x")],
            3,
        )
        .unwrap();
        let x = a.moment(&MomentKey::new("file", "/x", "")).unwrap();
        assert_eq!((x.visits, x.first_at, x.last_at), (2, 10, 12));
        let y = a.moment(&MomentKey::new("file", "/y", "")).unwrap();
        assert_eq!(
            (y.edits, y.dwell_ms, y.meta.as_str()),
            (2, 500, "{\"line\":4}")
        );
        // A delta with no meta keeps the row's.
        a.flush_moments(&[visit("file", "/y", 13)], &[ring(13, "/y")], 3)
            .unwrap();
        let y = a.moment(&MomentKey::new("file", "/y", "")).unwrap();
        assert_eq!((y.visits, y.meta.as_str()), (1, "{\"line\":4}"));
        // The ring: four rows pushed, the cap is three, the oldest went.
        let r = a.recent(10);
        assert_eq!(
            r.iter().map(|r| r.at).collect::<Vec<_>>(),
            [13, 12, 11],
            "{r:?}"
        );
        // A text: its bytes in the row, its header out of a query.
        a.flush_moments(
            &[(
                MomentKey::new("text", "h1", ""),
                MomentDelta {
                    first_at: 20,
                    last_at: 20,
                    text: Some(b"first line\nsecond\n".to_vec()),
                    ..Default::default()
                },
            )],
            &[],
            3,
        )
        .unwrap();
        let rows = a.moments(&MomentQuery {
            kind: Some("text"),
            ..Default::default()
        });
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].text_len, Some(18));
        assert_eq!(rows[0].text_head.as_deref(), Some("first line"));
        assert_eq!(
            a.moment_text(&MomentKey::new("text", "h1", "")).as_deref(),
            Some(&b"first line\nsecond\n"[..])
        );
        assert_eq!(a.text_bytes(), 18);
        // Newest first across kinds; a kind, a subject, a since.
        let all = a.moments(&MomentQuery::default());
        assert_eq!(
            all.iter()
                .map(|r| r.key.subject.as_str())
                .collect::<Vec<_>>(),
            ["h1", "/y", "/x"]
        );
        assert_eq!(
            a.moments(&MomentQuery {
                since: Some(13),
                ..Default::default()
            })
            .len(),
            2
        );
        assert_eq!(
            a.moments(&MomentQuery {
                subject: Some("/x"),
                limit: 5,
                ..Default::default()
            })
            .len(),
            1
        );
        // Pinned: the ordinal orders the pins.
        a.set_pinned(&MomentKey::new("file", "/x", ""), 2).unwrap();
        a.set_pinned(&MomentKey::new("file", "/y", ""), 1).unwrap();
        let pins = a.moments(&MomentQuery {
            pinned: true,
            ..Default::default()
        });
        assert_eq!(
            pins.iter()
                .map(|r| r.key.subject.as_str())
                .collect::<Vec<_>>(),
            ["/y", "/x"]
        );
        // Forgetting a file moment takes its ring rows and its history.
        a.save_history("file:/x", b"draft", "{}", false).unwrap();
        a.forget_moment(&MomentKey::new("file", "/x", "")).unwrap();
        assert!(a.moment(&MomentKey::new("file", "/x", "")).is_none());
        assert!(a.recent(10).iter().all(|r| r.key.subject != "/x"));
        assert!(a.load_history("file:/x").is_none());
        assert!(a.moments_bytes() > 0);
        a.clear_moments().unwrap();
        assert!(a.moments(&MomentQuery::default()).is_empty());
        assert_eq!(a.recent_len(), 0);
        drop((a, b));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A db from before: its `oldfiles` become `file` moments with
    /// their line and the table goes; a history without a moment gets
    /// one at its `saved_at`; both once.
    #[test]
    fn an_old_db_migrates() {
        let dir = std::env::temp_dir().join(format!("kawoosh-store-mig-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("old.db");
        let _ = std::fs::remove_file(&db);
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE oldfiles (path TEXT PRIMARY KEY, opened_at INTEGER NOT NULL, line INTEGER NOT NULL);
                 INSERT INTO oldfiles VALUES ('/old/a.rs', 1000, 7);
                 CREATE TABLE histories (key TEXT PRIMARY KEY, text BLOB NOT NULL, meta TEXT NOT NULL,
                     saved_at INTEGER NOT NULL, clean INTEGER NOT NULL DEFAULT 0);
                 INSERT INTO histories VALUES ('file:/old/b.rs', X'62', '{}', 2000, 0);
                 INSERT INTO histories VALUES ('scratch:3', X'73', '{}', 3000, 0);",
            )
            .unwrap();
        }
        let s = Store::open(&db).unwrap();
        let a = s.moment(&MomentKey::new("file", "/old/a.rs", "")).unwrap();
        assert_eq!(
            (a.visits, a.last_at, a.meta.as_str()),
            (1, 1000, "{\"line\":7}")
        );
        let b = s.moment(&MomentKey::new("file", "/old/b.rs", "")).unwrap();
        assert_eq!((b.visits, b.last_at), (0, 2000));
        let c = s
            .moment(&MomentKey::new("scratch", "scratch:3", ""))
            .unwrap();
        assert_eq!(c.last_at, 3000);
        assert!(
            s.conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name = 'oldfiles'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap()
                == 0
        );
        // Opened again: nothing doubles.
        drop(s);
        let s = Store::open(&db).unwrap();
        assert_eq!(s.moments(&MomentQuery::default()).len(), 3);
        assert_eq!(
            s.moment(&MomentKey::new("file", "/old/a.rs", ""))
                .unwrap()
                .visits,
            1
        );
        // A history whose moment lives under a workspace is not
        // twinned with an empty row under none at the next open.
        s.forget_moment(&MomentKey::new("file", "/old/b.rs", ""))
            .unwrap();
        s.save_history("file:/old/b.rs", b"b", "{}", false).unwrap();
        s.flush_moments(&[visit("file", "/old/b.rs", 5000)], &[], 10)
            .unwrap();
        let ws = MomentKey::new("file", "/old/b.rs", "/old");
        s.flush_moments(
            &[(
                ws.clone(),
                MomentDelta {
                    visits: 1,
                    first_at: 6000,
                    last_at: 6000,
                    ..Default::default()
                },
            )],
            &[],
            10,
        )
        .unwrap();
        s.forget_moment(&MomentKey::new("file", "/old/b.rs", ""))
            .unwrap();
        assert!(
            s.load_history("file:/old/b.rs").is_some(),
            "the workspace's row still owns the history"
        );
        drop(s);
        let s = Store::open(&db).unwrap();
        let b = s.moments(&MomentQuery {
            subject: Some("/old/b.rs"),
            ..Default::default()
        });
        assert_eq!(b.len(), 1, "{b:?}");
        assert_eq!(b[0].key.workspace, "/old");
        // The last row of the subject takes the history with it.
        s.forget_moment(&ws).unwrap();
        assert!(s.load_history("file:/old/b.rs").is_none());
        drop(s);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A flush merges a delta's meta over the row's: the engine's caret
    /// line and a plugin's keys keep out of each other's way.
    #[test]
    fn meta_is_merged_not_replaced() {
        let s = Store::in_memory().unwrap();
        let k = MomentKey::new("file", "/m", "");
        let delta = |meta: &str| MomentDelta {
            first_at: 1,
            last_at: 1,
            meta: Some(meta.into()),
            ..Default::default()
        };
        s.flush_moments(&[(k.clone(), delta("{\"from\":\"/old\"}"))], &[], 10)
            .unwrap();
        s.flush_moments(&[(k.clone(), delta("{\"line\":4}"))], &[], 10)
            .unwrap();
        let meta: serde_json::Value = serde_json::from_str(&s.moment(&k).unwrap().meta).unwrap();
        assert_eq!(meta["from"], "/old");
        assert_eq!(meta["line"], 4);
        assert_eq!(meta_line(&meta.to_string()), 4);
        assert_eq!(
            merge_meta("{\"a\":1,\"b\":2}", "{\"b\":3}"),
            "{\"a\":1,\"b\":3}"
        );
        assert_eq!(merge_meta("bad", "{\"b\":3}"), "{\"b\":3}");
    }

    /// A writer that meets another connection's lock gets `SQLITE_BUSY`
    /// after [`BUSY_MS`], not a hang and not a partial write.
    #[test]
    fn a_locked_db_says_busy() {
        let dir = std::env::temp_dir().join(format!("kawoosh-store-busy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("busy.db");
        let _ = std::fs::remove_file(&db);
        let s = Store::open(&db).unwrap();
        let other = Connection::open(&db).unwrap();
        other.execute_batch("BEGIN IMMEDIATE").unwrap();
        let t = std::time::Instant::now();
        let r = s.flush_moments(&[visit("file", "/x", 1)], &[], 10);
        assert!(r.is_err(), "busy");
        assert!(t.elapsed() >= std::time::Duration::from_millis(BUSY_MS - 50));
        other.execute_batch("COMMIT").unwrap();
        assert_eq!(
            s.flush_moments(&[visit("file", "/x", 1)], &[], 10).unwrap(),
            1,
            "the row made"
        );
        assert_eq!(
            s.moment(&MomentKey::new("file", "/x", "")).unwrap().visits,
            1
        );
        assert_eq!(
            s.flush_moments(&[visit("file", "/x", 2)], &[], 10).unwrap(),
            0,
            "a visit to a row the table has makes none"
        );
        assert_eq!(
            s.flush_moments(&[visit("file", "/x", 3), visit("file", "/y", 3)], &[], 10)
                .unwrap(),
            1
        );
        drop((s, other));
        std::fs::remove_dir_all(&dir).ok();
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
        s.vacuum().unwrap();
    }
}
