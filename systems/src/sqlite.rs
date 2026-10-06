//! A user's SQLite database, for the database pane
//! (docs/design/sqlite.md, `kawoosh/lua/sqlite.lua` over
//! `kawoosh.sqlite`): what tables it has, a query run, a value as it
//! is. Every call opens the file for itself and closes it after
//! (Decision 2) — nothing is held across frames, and what comes back
//! is what the file says now. Run on the io thread; a long query holds
//! that thread, not the frame.

use std::path::Path;
use std::time::{Duration, Instant};

use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};

/// What the file's first sixteen bytes say when it is a database.
const MAGIC: &[u8; 16] = b"SQLite format 3\0";
const BUSY: Duration = Duration::from_secs(1);

/// A value as SQLite holds it.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl From<ValueRef<'_>> for Value {
    fn from(v: ValueRef<'_>) -> Self {
        match v {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(i) => Value::Integer(i),
            ValueRef::Real(r) => Value::Real(r),
            // Text that is not UTF-8 is kept as what it is: bytes.
            ValueRef::Text(t) => match std::str::from_utf8(t) {
                Ok(s) => Value::Text(s.to_string()),
                Err(_) => Value::Blob(t.to_vec()),
            },
            ValueRef::Blob(b) => Value::Blob(b.to_vec()),
        }
    }
}

impl rusqlite::ToSql for Value {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        use rusqlite::types::ToSqlOutput;
        Ok(match self {
            Value::Null => ToSqlOutput::Borrowed(ValueRef::Null),
            Value::Integer(i) => ToSqlOutput::Borrowed(ValueRef::Integer(*i)),
            Value::Real(r) => ToSqlOutput::Borrowed(ValueRef::Real(*r)),
            Value::Text(t) => ToSqlOutput::Borrowed(ValueRef::Text(t.as_bytes())),
            Value::Blob(b) => ToSqlOutput::Borrowed(ValueRef::Blob(b)),
        })
    }
}

/// What a query gave: the columns and rows of the last statement that
/// had columns, how many rows every statement changed, whether the
/// rows were cut at the cap, and the time taken.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub truncated: bool,
    pub changes: usize,
    pub ms: f64,
}

/// A column of a table, as `PRAGMA table_info` says.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub name: String,
    /// The declared type, `""` when none.
    pub kind: String,
    pub notnull: bool,
    /// Its place in the primary key, from 1; 0 when not in it.
    pub pk: i64,
    pub default: Option<String>,
}

/// A table or a view.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub name: String,
    /// `table` or `view`.
    pub kind: String,
    /// Its row count; `None` when the count could not be read (a
    /// view over something missing).
    pub rows: Option<i64>,
    pub without_rowid: bool,
    pub columns: Vec<Column>,
}

/// What a database has: its tables and views in the order
/// `sqlite_master` lists them, and the file's size as its pages say.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Schema {
    pub tables: Vec<Table>,
    pub bytes: u64,
}

/// Whether the file's head says it is a database — sixteen bytes
/// read, the rest left alone.
pub fn is_sqlite(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0u8; 16];
    f.read_exact(&mut head).is_ok() && &head == MAGIC
}

fn open(path: &Path) -> Result<Connection, String> {
    if !path.is_file() {
        return Err(format!("{}: no such file", path.display()));
    }
    // Without CREATE: a path that is not there is an error, not a new
    // database. A file the user cannot write is opened read-only by
    // SQLite's own rule.
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("{}: {e}", path.display()))?;
    conn.busy_timeout(BUSY).map_err(|e| e.to_string())?;
    Ok(conn)
}

/// `sql` run against the database at `path`: every statement in it in
/// turn, `params` bound by position to each, the rows of the last
/// statement with columns kept up to `cap` of them (`truncated` when
/// there were more), the changes summed. An error is SQLite's words.
pub fn query(path: &Path, sql: &str, params: &[Value], cap: usize) -> Result<Rows, String> {
    let started = Instant::now();
    let conn = open(path)?;
    let mut out = Rows::default();
    let mut batch = rusqlite::Batch::new(&conn, sql);
    let mut any = false;
    while let Some(mut stmt) = batch.next().map_err(|e| e.to_string())? {
        any = true;
        let bound: Vec<&dyn rusqlite::ToSql> = params
            .iter()
            .take(stmt.parameter_count())
            .map(|v| v as &dyn rusqlite::ToSql)
            .collect();
        if stmt.column_count() == 0 {
            out.changes += stmt.execute(bound.as_slice()).map_err(|e| e.to_string())?;
            continue;
        }
        let columns: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let n = columns.len();
        let mut rows = Vec::new();
        let mut truncated = false;
        let mut got = stmt.query(bound.as_slice()).map_err(|e| e.to_string())?;
        while let Some(row) = got.next().map_err(|e| e.to_string())? {
            if rows.len() >= cap {
                truncated = true;
                break;
            }
            let mut vals = Vec::with_capacity(n);
            for i in 0..n {
                vals.push(Value::from(row.get_ref(i).map_err(|e| e.to_string())?));
            }
            rows.push(vals);
        }
        drop(got);
        // A statement that writes through a RETURNING clause has rows
        // and changes both.
        out.changes += conn.changes() as usize * usize::from(!stmt.readonly());
        out.columns = columns;
        out.rows = rows;
        out.truncated = truncated;
    }
    if !any {
        return Err("nothing to run".into());
    }
    out.ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(out)
}

/// A name quoted as an identifier: `"a""b"`.
pub fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// The database's tables and views, each with its columns and its row
/// count, and the file's size.
pub fn schema(path: &Path) -> Result<Schema, String> {
    let conn = open(path)?;
    let mut out = Schema::default();
    let page: i64 = conn
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .unwrap_or(0);
    let count: i64 = conn
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap_or(0);
    out.bytes = (page.max(0) as u64) * (count.max(0) as u64);
    let mut stmt = conn
        .prepare(
            "SELECT type, name, sql FROM sqlite_master
             WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%'
             ORDER BY type, name",
        )
        .map_err(|e| e.to_string())?;
    let listed: Vec<(String, String, Option<String>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    for (kind, name, sql) in listed {
        let without_rowid = sql
            .as_deref()
            .map(|s| s.to_ascii_uppercase().contains("WITHOUT ROWID"))
            .unwrap_or(false);
        let rows = conn
            .query_row(&format!("SELECT count(*) FROM {}", quote(&name)), [], |r| {
                r.get::<_, i64>(0)
            })
            .ok();
        let mut columns = Vec::new();
        if let Ok(mut info) = conn.prepare(&format!("PRAGMA table_info({})", quote(&name))) {
            let got = info.query_map([], |r| {
                Ok(Column {
                    name: r.get(1)?,
                    kind: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    notnull: r.get::<_, i64>(3)? != 0,
                    default: r.get(4)?,
                    pk: r.get(5)?,
                })
            });
            if let Ok(got) = got {
                columns = got.flatten().collect();
            }
        }
        out.tables.push(Table {
            name,
            kind,
            rows,
            without_rowid,
            columns,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kawoosh-sqlite-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL, age INTEGER, note);
             INSERT INTO people (name, age, note) VALUES ('ann', 31, NULL), ('bob', 44, x'0102');
             CREATE TABLE pairs (a TEXT, b TEXT, PRIMARY KEY (a, b)) WITHOUT ROWID;
             CREATE VIEW adults AS SELECT name FROM people WHERE age >= 40;",
        )
        .unwrap();
        path
    }

    #[test]
    fn the_head_says_what_it_is_and_the_schema_lists_the_tables() {
        let path = fixture("schema");
        assert!(is_sqlite(&path));
        assert!(!is_sqlite(&path.with_file_name("none.db")));
        let s = schema(&path).unwrap();
        let names: Vec<(&str, &str, Option<i64>)> = s
            .tables
            .iter()
            .map(|t| (t.kind.as_str(), t.name.as_str(), t.rows))
            .collect();
        assert_eq!(
            names,
            vec![
                ("table", "pairs", Some(0)),
                ("table", "people", Some(2)),
                ("view", "adults", Some(1)),
            ]
        );
        let people = &s.tables[1];
        assert!(!people.without_rowid);
        assert!(s.tables[0].without_rowid);
        let cols: Vec<(&str, &str, i64)> = people
            .columns
            .iter()
            .map(|c| (c.name.as_str(), c.kind.as_str(), c.pk))
            .collect();
        assert_eq!(
            cols,
            vec![
                ("id", "INTEGER", 1),
                ("name", "TEXT", 0),
                ("age", "INTEGER", 0),
                ("note", "", 0)
            ]
        );
        assert!(people.columns[1].notnull);
        assert!(s.bytes > 0);
    }

    #[test]
    fn a_query_gives_typed_rows_runs_several_statements_and_is_capped() {
        let path = fixture("query");
        let r = query(
            &path,
            "SELECT id, name, age, note FROM people ORDER BY id",
            &[],
            10,
        )
        .unwrap();
        assert_eq!(r.columns, vec!["id", "name", "age", "note"]);
        assert_eq!(
            r.rows[0],
            vec![
                Value::Integer(1),
                Value::Text("ann".into()),
                Value::Integer(31),
                Value::Null
            ]
        );
        assert_eq!(r.rows[1][3], Value::Blob(vec![1, 2]));
        assert!(!r.truncated);
        assert_eq!(r.changes, 0);
        // The cap.
        let r = query(&path, "SELECT * FROM people", &[], 1).unwrap();
        assert_eq!(r.rows.len(), 1);
        assert!(r.truncated);
        // Several statements: the rows are the last one's with columns,
        // the changes every one's.
        let r = query(
            &path,
            "UPDATE people SET age = age + 1; SELECT age FROM people ORDER BY id; INSERT INTO people (name) VALUES ('cy')",
            &[],
            10,
        )
        .unwrap();
        assert_eq!(r.changes, 3);
        assert_eq!(
            r.rows,
            vec![vec![Value::Integer(32)], vec![Value::Integer(45)]]
        );
        // Parameters, and affinity typing a text into an INTEGER column.
        let r = query(
            &path,
            "UPDATE people SET age = ?1 WHERE rowid = ?2",
            &[Value::Text("50".into()), Value::Integer(1)],
            10,
        )
        .unwrap();
        assert_eq!(r.changes, 1);
        let r = query(&path, "SELECT age FROM people WHERE id = 1", &[], 10).unwrap();
        assert_eq!(r.rows[0][0], Value::Integer(50));
        // An error is SQLite's words; nothing to run is said.
        let e = query(&path, "SELEC 1", &[], 10).unwrap_err();
        assert!(e.contains("syntax error"), "{e}");
        assert_eq!(query(&path, "  ", &[], 10).unwrap_err(), "nothing to run");
        // A path that is not there is not made.
        let gone = path.with_file_name("none.db");
        assert!(query(&gone, "SELECT 1", &[], 1).is_err());
        assert!(!gone.exists());
    }
}
