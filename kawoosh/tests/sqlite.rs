//! The database pane (`kawoosh/lua/sqlite.lua` over `kawoosh.sqlite`,
//! docs/design/sqlite.md): a file's tables listed with their counts,
//! a table browsed as a grid the keys walk cell by cell, a query run
//! from the line and its rows or its error shown, a cell changed by
//! one UPDATE and put back, a table sorted by a column — and a
//! database opened here rather than as bytes.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

fn lua(app: &mut Kawoosh, src: &str) -> String {
    app.run_lua_source("t", src);
    app.ed.message.clone()
}

/// `focus table row col` of the pane with the keyboard.
fn shown(app: &mut Kawoosh) -> String {
    lua(
        app,
        r#"local s = kawoosh.sqlite_pane.state()
           kawoosh.echo(s and string.format("%s %s %d %d", s.focus, tostring(s.table), s.cursor.row, s.cursor.col) or "nil")"#,
    )
}

fn field(app: &mut Kawoosh, name: &str) -> String {
    lua(
        app,
        &format!(
            r#"local s = kawoosh.sqlite_pane.state()
               kawoosh.echo(s and tostring(s.{name}) or "nil")"#
        ),
    )
}

/// The grid's rows as drawn, the header first, trailing blanks cut.
fn rows(app: &mut Kawoosh) -> Vec<String> {
    lua(
        app,
        r#"kawoosh.echo(table.concat(kawoosh.sqlite_pane.state().lines, "\n"))"#,
    )
    .lines()
    .map(|l| l.trim_end().to_string())
    .filter(|l| !l.is_empty())
    .collect()
}

fn fixture(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("kawoosh-sqlite-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let db = root.join("shop.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL, price REAL, qty INTEGER, note);
         INSERT INTO items (name, price, qty, note) VALUES
           ('apple', 1.5, 10, NULL), ('pear', 2.25, 3, 'ripe'), ('fig', 4, 0, x'00ff');
         CREATE TABLE codes (a TEXT, b TEXT, v INTEGER, PRIMARY KEY (a, b)) WITHOUT ROWID;
         INSERT INTO codes VALUES ('x', 'y', 1);
         CREATE VIEW cheap AS SELECT name FROM items WHERE price < 3;",
    )
    .unwrap();
    drop(conn);
    std::fs::write(root.join("plain.txt"), "only text\n").unwrap();
    let root = kawoosh_systems::fs::canonicalize(&root).unwrap();
    (root.clone(), root.join("shop.db"))
}

/// Frames until the pane has read the file and browsed the first table.
fn settle(d: &mut Drive, app: &mut Kawoosh) {
    for _ in 0..12 {
        d.frame(app);
        if field(app, "rows") != "0" && field(app, "loading") == "false" {
            break;
        }
    }
    d.frame(app);
}

fn open(name: &str) -> (Drive, Kawoosh, std::path::PathBuf, std::path::PathBuf) {
    let (root, db) = fixture(name);
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(1600.0, 700.0);
    d.extension("lua", ext).unwrap();
    app.open_store(Some(&root.join("state.db")));
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("sqlite {}", db.display()));
    settle(&mut d, &mut app);
    (d, app, root, db)
}

fn value(db: &std::path::Path, sql: &str) -> String {
    let conn = rusqlite::Connection::open(db).unwrap();
    conn.query_row(sql, [], |r| {
        Ok(match r.get_ref(0).unwrap() {
            rusqlite::types::ValueRef::Null => "NULL".to_string(),
            rusqlite::types::ValueRef::Integer(i) => i.to_string(),
            rusqlite::types::ValueRef::Real(f) => f.to_string(),
            rusqlite::types::ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
            rusqlite::types::ValueRef::Blob(b) => format!("blob {}", b.len()),
        })
    })
    .unwrap()
}

#[test]
fn the_tables_are_listed_and_the_first_is_browsed_as_a_grid_the_keys_walk() {
    let (mut d, mut app, _root, _db) = open("walk");
    // The tables and the view, each counted; `codes` first by name.
    assert_eq!(
        lua(
            &mut app,
            r#"kawoosh.echo(table.concat(kawoosh.sqlite_pane.state().tables, " "))"#
        ),
        "codes items cheap"
    );
    assert_eq!(shown(&mut app), "tables codes 1 1");
    // Into the grid, down to items, browse it.
    d.press(&mut app, "j<CR>");
    settle(&mut d, &mut app);
    assert_eq!(shown(&mut app), "grid items 1 1");
    assert_eq!(field(&mut app, "total"), "3");
    let grid = rows(&mut app);
    assert_eq!(grid[0], "     id │ name  │ price │ qty │ note");
    assert_eq!(grid[1], "  1   1 │ apple │   1.5 │  10 │ NULL");
    assert_eq!(grid[2], "  2   2 │ pear  │  2.25 │   3 │ ripe");
    assert_eq!(grid[3], "  3   3 │ fig   │     4 │   0 │ x'00ff' 2 B");
    assert_eq!(grid.len(), 4, "the rowid column is kept, not shown");
    // Cells.
    d.press(&mut app, "l");
    assert_eq!(shown(&mut app), "grid items 1 2");
    assert_eq!(field(&mut app, "value"), "apple");
    d.press(&mut app, "jj");
    assert_eq!(field(&mut app, "value"), "fig");
    d.press(&mut app, "j");
    assert_eq!(shown(&mut app), "grid items 3 2", "not past the last row");
    d.press(&mut app, "$");
    assert_eq!(shown(&mut app), "grid items 3 5");
    d.press(&mut app, "0");
    assert_eq!(shown(&mut app), "grid items 3 1");
    d.press(&mut app, "gl");
    assert_eq!(shown(&mut app), "grid items 3 5", "helix's `gl`");
    d.press(&mut app, "gh");
    assert_eq!(shown(&mut app), "grid items 3 1", "helix's `gh`");
    d.press(&mut app, "gg");
    assert_eq!(shown(&mut app), "grid items 1 1");
    d.press(&mut app, "G");
    assert_eq!(shown(&mut app), "grid items 3 1");
    d.press(&mut app, "3l");
    assert_eq!(shown(&mut app), "grid items 3 4");
    // Back to the tables and down to the view.
    d.press(&mut app, "<Tab>");
    assert_eq!(shown(&mut app), "tables items 3 4");
    d.press(&mut app, "G<CR>");
    settle(&mut d, &mut app);
    assert_eq!(shown(&mut app), "grid cheap 1 1");
    assert_eq!(rows(&mut app)[1], "  1  apple");
    // A cell copied, a row copied.
    d.press(&mut app, "<Tab>k<CR>");
    settle(&mut d, &mut app);
    d.press(&mut app, "jly");
    assert_eq!(app.ed.message, "the cell copied");
    assert_eq!(app.ed.memory.head().unwrap().text, "pear");
    d.press(&mut app, "Y");
    assert_eq!(app.ed.memory.head().unwrap().text, "2\tpear\t2.25\t3\tripe");
}

#[test]
fn a_query_runs_from_the_line_and_from_the_command_line_and_an_error_is_said() {
    let (mut d, mut app, _root, _db) = open("query");
    // From the command line.
    ex(
        &mut d,
        &mut app,
        "sqlite query SELECT name, qty * 2 AS twice FROM items WHERE qty > 0 ORDER BY name",
    );
    settle(&mut d, &mut app);
    assert_eq!(field(&mut app, "table"), "nil");
    let grid = rows(&mut app);
    assert_eq!(grid[0], "     name  │ twice");
    assert_eq!(grid[1], "  1  apple │    20");
    assert_eq!(grid[2], "  2  pear  │     6");
    assert!(
        field(&mut app, "status").starts_with("2 rows · "),
        "{}",
        field(&mut app, "status")
    );
    // From the line: `i` puts the keys in it, Enter runs.
    d.press(&mut app, "i");
    d.frame(&mut app);
    d.keys(&mut app, "SELECT count(*) AS n FROM items");
    d.press(&mut app, "<CR>");
    settle(&mut d, &mut app);
    assert_eq!(rows(&mut app)[1], "  1  3");
    assert_eq!(shown(&mut app), "grid nil 1 1");
    // An error is SQLite's words, in the foot.
    ex(&mut d, &mut app, "sqlite query SELEC nothing");
    settle(&mut d, &mut app);
    assert!(
        field(&mut app, "error").contains("syntax error"),
        "{}",
        field(&mut app, "error")
    );
    // The lines run are kept: <C-p> walks them in the line.
    d.press(&mut app, "i<C-p>");
    d.frame(&mut app);
    assert_eq!(
        lua(
            &mut app,
            r#"kawoosh.echo(kawoosh.field_text("sqlite", "sql"))"#
        ),
        "SELEC nothing"
    );
    d.press(&mut app, "<C-p>");
    assert_eq!(
        lua(
            &mut app,
            r#"kawoosh.echo(kawoosh.field_text("sqlite", "sql"))"#
        ),
        "SELECT count(*) AS n FROM items"
    );
    d.press(&mut app, "<C-n><C-n>");
    assert_eq!(
        lua(
            &mut app,
            r#"kawoosh.echo(kawoosh.field_text("sqlite", "sql"))"#
        ),
        ""
    );
    // A statement without rows says its changes, and the table shown
    // before is read again with its count.
    d.press(&mut app, "<Esc><Esc>");
    ex(&mut d, &mut app, "sqlite browse items");
    settle(&mut d, &mut app);
    ex(
        &mut d,
        &mut app,
        "sqlite query INSERT INTO items (name) VALUES ('kiwi')",
    );
    settle(&mut d, &mut app);
    assert!(
        field(&mut app, "status").starts_with("1 changed"),
        "{}",
        field(&mut app, "status")
    );
    assert_eq!(field(&mut app, "table"), "items");
    assert_eq!(field(&mut app, "total"), "4");
    assert_eq!(rows(&mut app)[4], "  4   4 │ kiwi  │  NULL │ NULL │ NULL");
}

#[test]
fn a_cell_is_changed_by_one_update_set_null_put_back_and_the_table_sorted() {
    let (mut d, mut app, _root, db) = open("edit");
    d.press(&mut app, "j<CR>");
    settle(&mut d, &mut app);
    // `c` puts the cell's value in the line; Enter writes it.
    d.press(&mut app, "jl");
    assert_eq!(field(&mut app, "value"), "pear");
    d.press(&mut app, "c");
    d.frame(&mut app);
    assert_eq!(field(&mut app, "editing"), "true");
    assert_eq!(
        lua(
            &mut app,
            r#"kawoosh.echo(kawoosh.field_text("sqlite", "sql"))"#
        ),
        "pear"
    );
    d.press(&mut app, "<C-w>");
    d.keys(&mut app, "plum");
    d.press(&mut app, "<CR>");
    settle(&mut d, &mut app);
    assert_eq!(field(&mut app, "editing"), "false");
    assert_eq!(value(&db, "SELECT name FROM items WHERE id = 2"), "plum");
    assert_eq!(rows(&mut app)[2], "  2   2 │ plum  │  2.25 │   3 │ ripe");
    assert_eq!(shown(&mut app), "grid items 2 2");
    assert_eq!(field(&mut app, "changes"), "1");
    // Text into an INTEGER column is typed by affinity.
    d.press(&mut app, "llc");
    d.frame(&mut app);
    d.press(&mut app, "<C-w>");
    d.keys(&mut app, "42");
    d.press(&mut app, "<CR>");
    settle(&mut d, &mut app);
    assert_eq!(
        value(&db, "SELECT typeof(qty) || qty FROM items WHERE id = 2"),
        "integer42"
    );
    // NULL, and the last change put back twice.
    d.press(&mut app, "x");
    settle(&mut d, &mut app);
    assert_eq!(value(&db, "SELECT qty FROM items WHERE id = 2"), "NULL");
    assert_eq!(rows(&mut app)[2], "  2   2 │ plum  │  2.25 │ NULL │ ripe");
    d.press(&mut app, "u");
    settle(&mut d, &mut app);
    assert_eq!(value(&db, "SELECT qty FROM items WHERE id = 2"), "42");
    d.press(&mut app, "u");
    settle(&mut d, &mut app);
    assert_eq!(value(&db, "SELECT qty FROM items WHERE id = 2"), "3");
    d.press(&mut app, "u");
    settle(&mut d, &mut app);
    assert_eq!(value(&db, "SELECT name FROM items WHERE id = 2"), "pear");
    d.press(&mut app, "u");
    assert_eq!(field(&mut app, "status"), "nothing to put back");
    // A table WITHOUT ROWID is keyed by its primary key.
    ex(&mut d, &mut app, "sqlite browse codes");
    settle(&mut d, &mut app);
    assert_eq!(rows(&mut app)[0], "     a │ b │ v");
    d.press(&mut app, "$c");
    d.frame(&mut app);
    d.press(&mut app, "<C-w>");
    d.keys(&mut app, "7");
    d.press(&mut app, "<CR>");
    settle(&mut d, &mut app);
    assert_eq!(
        value(&db, "SELECT v FROM codes WHERE a = 'x' AND b = 'y'"),
        "7"
    );
    assert_eq!(rows(&mut app)[1], "  1  x │ y │ 7");
    // A view's cell, and a query's, refuse.
    ex(&mut d, &mut app, "sqlite browse cheap");
    settle(&mut d, &mut app);
    d.press(&mut app, "c");
    assert_eq!(field(&mut app, "error"), "a view's rows cannot be changed");
    ex(&mut d, &mut app, "sqlite query SELECT * FROM items");
    settle(&mut d, &mut app);
    d.press(&mut app, "x");
    assert_eq!(
        field(&mut app, "error"),
        "a query's rows: browse the table to change one"
    );
    // Sorted by a column, again the other way.
    ex(&mut d, &mut app, "sqlite browse items");
    settle(&mut d, &mut app);
    d.press(&mut app, "llo");
    settle(&mut d, &mut app);
    assert_eq!(field(&mut app, "order"), "price");
    let grid = rows(&mut app);
    assert!(grid[0].contains("price ▴"), "{}", grid[0]);
    assert!(grid[1].contains("apple"), "{}", grid[1]);
    d.press(&mut app, "o");
    settle(&mut d, &mut app);
    assert_eq!(field(&mut app, "desc"), "true");
    assert!(rows(&mut app)[1].contains("fig"), "{}", rows(&mut app)[1]);
}

#[test]
fn a_database_opens_here_rather_than_as_bytes_and_t_opens_the_bytes() {
    let (mut d, mut app, root, db) = open("open");
    d.press(&mut app, "q");
    d.frame(&mut app);
    assert_eq!(shown(&mut app), "nil");
    ex(&mut d, &mut app, &format!("e {}", db.display()));
    settle(&mut d, &mut app);
    assert_eq!(shown(&mut app), "tables codes 1 1");
    assert_eq!(
        lua(&mut app, "kawoosh.echo(tostring(kawoosh.hex.state()))"),
        "nil",
        "not the bytes pane"
    );
    // `t`: the bytes after all.
    d.press(&mut app, "t");
    d.frame(&mut app);
    d.frame(&mut app);
    assert_ne!(
        lua(&mut app, "kawoosh.echo(tostring(kawoosh.hex.state()))"),
        "nil"
    );
    // A text file is a buffer, as ever.
    ex(
        &mut d,
        &mut app,
        &format!("e {}", root.join("plain.txt").display()),
    );
    d.frame(&mut app);
    assert_eq!(
        lua(&mut app, "kawoosh.echo(kawoosh.buf.text())"),
        "only text\n"
    );
    // The setting off: the bytes pane takes it.
    ex(&mut d, &mut app, "set sqlite.open=false");
    ex(&mut d, &mut app, &format!("e {}", db.display()));
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(
        lua(&mut app, r#"kawoosh.echo(kawoosh.hex.state().path)"#),
        db.display().to_string()
    );
}

/// A table longer than a page pages in as the cursor nears its last
/// row fetched, and a page that is the table's last says so: the page
/// asked for one row past itself, which the cap leaves out. Before, a
/// page of `sqlite.rows` asked with that cap was never cut, so the grid
/// stopped at its first page.
#[test]
fn a_table_longer_than_a_page_pages_in_to_its_last_row() {
    let (mut d, mut app, _root, db) = open("pages");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE many (n INTEGER);
         WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c WHERE n < 25)
         INSERT INTO many SELECT n FROM c;",
    )
    .unwrap();
    drop(conn);
    ex(&mut d, &mut app, "set sqlite.rows=10");
    d.press(&mut app, "r");
    settle(&mut d, &mut app);
    lua(&mut app, "kawoosh.sqlite_pane.browse('many')");
    settle(&mut d, &mut app);
    assert_eq!(field(&mut app, "rows"), "10", "a page");
    assert_eq!(field(&mut app, "more"), "true", "and more past it");
    if shown(&mut app).starts_with("tables") {
        d.press(&mut app, "<Tab>");
    }
    assert!(shown(&mut app).starts_with("grid many"));
    for _ in 0..6 {
        d.press(&mut app, "G");
        settle(&mut d, &mut app);
    }
    assert_eq!(field(&mut app, "rows"), "25", "every row, a page at a time");
    assert_eq!(field(&mut app, "more"), "false", "the last page says so");
    assert_eq!(field(&mut app, "value"), "25");
}

/// A blob past 4 KiB comes to the grid cut — its first bytes and its
/// size, not a megabyte a cell — and is read whole again where all of
/// it is wanted: `y` copies every byte, and `x` then `u` puts back the
/// whole blob, never the cut one.
#[test]
fn a_long_blob_is_shown_cut_and_copied_and_put_back_whole() {
    let (mut d, mut app, _root, db) = open("blob");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE files (id INTEGER PRIMARY KEY, data BLOB);
         INSERT INTO files (data) VALUES (zeroblob(10000));",
    )
    .unwrap();
    drop(conn);
    d.press(&mut app, "r");
    settle(&mut d, &mut app);
    lua(&mut app, "kawoosh.sqlite_pane.browse('files')");
    settle(&mut d, &mut app);
    if shown(&mut app).starts_with("tables") {
        d.press(&mut app, "<Tab>");
    }
    d.press(&mut app, "l");
    assert_eq!(shown(&mut app), "grid files 1 2");
    assert_eq!(field(&mut app, "value"), "x'0000000000000000…' 9.8 KB");
    d.press(&mut app, "y");
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(app.ed.message, "the cell copied");
    assert_eq!(
        app.ed.memory.head().unwrap().text,
        "0".repeat(20000),
        "every byte, in hex"
    );
    d.press(&mut app, "x");
    settle(&mut d, &mut app);
    assert_eq!(value(&db, "SELECT data FROM files"), "NULL");
    d.press(&mut app, "u");
    settle(&mut d, &mut app);
    assert_eq!(
        value(&db, "SELECT data FROM files"),
        "blob 10000",
        "the whole blob put back"
    );
}
