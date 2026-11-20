//! Checks against real MariaDB and MySQL servers.
//!
//! These need the local test servers from `tests/servers/` to be running, so
//! they are ignored by default. Start the servers, then run:
//!
//!     cargo test -p sqltool-core --test real_servers -- --ignored
//!
//! They cover what the fake server cannot: real metadata, statements that
//! change rows, stored procedures returning several result sets, and the row
//! limit interrupting a real query.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sqltool_core::edit::{self, CellChange, RowEdit};
use sqltool_core::table::{self, ColumnSpec, ForeignKeySpec, IndexSpec, TableSpec};
use sqltool_core::{ddl, split, CaSource, ColumnKind, ConnectProfile, ObjectKind, ResultStore, RunEvent, Session};

fn local_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/servers/.local")
}

fn env_file(name: &str) -> HashMap<String, String> {
    let path = local_dir().join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} is missing ({e}); run the scripts in tests/servers first", path.display()));
    text.lines().filter_map(|line| line.split_once('=')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect()
}

/// MariaDB, logging in through PAM with the cleartext plugin over verified TLS.
fn mariadb() -> ConnectProfile {
    let env = env_file("mariadb.env");
    let mut p = ConnectProfile::new("127.0.0.1", 3307, "sqltool_pam", env["PAM_PASSWORD"].clone().into());
    p.ca = Some(CaSource::File(local_dir().join("mariadb-ca.pem")));
    p.allow_cleartext_auth = true;
    p.database = Some("sqltool_test".into());
    p
}

/// MySQL 8.4, `caching_sha2_password` over verified TLS.
fn mysql84() -> ConnectProfile {
    let env = env_file("mysql84.env");
    let mut p = ConnectProfile::new("127.0.0.1", 3308, "sqltool_sha2", env["SQL_PASSWORD"].clone().into());
    p.ca = Some(CaSource::File(local_dir().join("mysql84-ca.pem")));
    p.database = Some("sqltool_test".into());
    p
}

/// Everything one statement produced, in order.
#[derive(Default)]
struct Outcome {
    sets: Vec<(Arc<ResultStore>, u64, bool)>,
    done: Vec<(u64, Option<u64>, String)>,
    order: Vec<&'static str>,
}

async fn run(session: &mut Session, sql: &str, max_rows: Option<u64>) -> Outcome {
    let mut out = Outcome::default();
    let mut current: Option<Arc<ResultStore>> = None;
    session
        .run(sql, max_rows, |event| match event {
            RunEvent::ResultSet { store } => {
                out.order.push("set");
                current = Some(store);
            }
            RunEvent::Progress { .. } => {}
            RunEvent::ResultDone { rows, truncated } => {
                out.order.push("set-done");
                out.sets.push((current.take().expect("result without a start"), rows, truncated));
            }
            RunEvent::Done { affected_rows, last_insert_id, info, .. } => {
                out.order.push("done");
                out.done.push((affected_rows, last_insert_id, info));
            }
        })
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
    out
}

fn cell(store: &ResultStore, row: u64, column: usize) -> Option<String> {
    store.cell(row, column).expect("cell exists").map(|bytes| String::from_utf8(bytes).unwrap())
}

async fn check_metadata(session: &mut Session, expect_procedure: bool) {
    let schemas = session.schemas().await.unwrap();
    assert!(schemas.iter().any(|s| s == "sqltool_test"), "schemas: {schemas:?}");
    assert!(schemas.iter().any(|s| s == "information_schema"));

    let objects = session.schema_objects("sqltool_test").await.unwrap();
    let people = objects.tables.iter().find(|t| t.name == "people").expect("people table");
    assert!(!people.is_view);
    let names = objects.tables.iter().find(|t| t.name == "people_names").expect("people_names view");
    assert!(names.is_view);
    if expect_procedure {
        let routine = objects.routines.iter().find(|r| r.name == "two_sets").expect("two_sets routine");
        assert_eq!(routine.kind, "PROCEDURE");
    }

    let columns = session.table_columns("sqltool_test", "people").await.unwrap();
    let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "name", "email", "born", "notes", "avatar", "profile", "created_at"]);
    assert_eq!(columns[0].key, "PRI");
    assert!(columns[0].extra.contains("auto_increment"));
    assert!(columns[0].column_type.starts_with("int"), "{}", columns[0].column_type);
    assert!(!columns[1].nullable);
    assert!(columns[2].nullable);
    assert_eq!(columns[1].key, "MUL");

    let details = session.table_details("sqltool_test", "people").await.unwrap().expect("details");
    assert!(!details.is_view);
    assert_eq!(details.engine.as_deref(), Some("InnoDB"));
    assert!(details.collation.unwrap().starts_with("utf8mb4"));
    assert!(session.table_details("sqltool_test", "people_names").await.unwrap().unwrap().is_view);
    assert_eq!(session.table_details("sqltool_test", "missing").await.unwrap(), None);

    let defaults = session.schema_defaults("sqltool_test").await.unwrap().expect("schema defaults");
    assert!(defaults.charset.starts_with("utf8mb"), "{defaults:?}");
    let charsets = session.charsets().await.unwrap();
    let utf8mb4 = charsets.iter().find(|c| c.name == "utf8mb4").expect("utf8mb4 is offered");
    let collations = session.collations("utf8mb4").await.unwrap();
    assert!(collations.contains(&utf8mb4.default_collation));
    assert!(session.collations("no'such").await.unwrap().is_empty());

    let table = session.show_create(ObjectKind::Table, "sqltool_test", "people").await.unwrap();
    assert!(table.starts_with("CREATE TABLE `people`"), "{table}");
    let view = session.show_create(ObjectKind::View, "sqltool_test", "people_names").await.unwrap();
    assert!(view.to_uppercase().contains("VIEW `PEOPLE_NAMES` AS"), "{view}");
    if expect_procedure {
        let procedure = session.show_create(ObjectKind::Procedure, "sqltool_test", "two_sets").await.unwrap();
        assert!(procedure.contains("PROCEDURE `two_sets`"), "{procedure}");
        // The alter script replaces the procedure in place (this is MariaDB).
        // Run as it is, it either works or, when the procedure belongs to
        // another account, is refused. Either way the procedure must survive.
        let script = ddl::alter_script(ObjectKind::Procedure, "sqltool_test", "two_sets", &procedure, true);
        let mut refused = false;
        for statement in sqltool_core::split::split(&script) {
            if let Err(e) = session.run(&statement.text, None, |_| {}).await {
                assert!(e.to_string().contains("1227"), "unexpected failure: {e}");
                refused = true;
            }
        }
        assert_eq!(run(session, "CALL sqltool_test.two_sets()", None).await.sets.len(), 2, "still there");

        if refused {
            // Without the DEFINER clause the same script creates it as this account.
            let start = script.find("DEFINER=").expect("definer clause");
            let end = start + script[start..].find(" PROCEDURE").expect("end of definer clause") + 1;
            let own = format!("{}{}", &script[..start], &script[end..]);
            for statement in sqltool_core::split::split(&own) {
                run(session, &statement.text, None).await;
            }
            assert_eq!(run(session, "CALL sqltool_test.two_sets()", None).await.sets.len(), 2);
        }
    }
    assert!(session.show_create(ObjectKind::Table, "sqltool_test", "missing").await.is_err());

    // A name that needs escaping must not break, or leak into, the query.
    assert!(session.schema_objects("no'such`schema").await.unwrap().tables.is_empty());
    assert!(session.table_columns("sqltool_test", "x' OR '1'='1").await.unwrap().is_empty());
}

async fn check_rows_and_kinds(session: &mut Session, json_is_text: bool) {
    let out = run(session, "SELECT id, name, born, profile, avatar, notes FROM sqltool_test.people ORDER BY id", None).await;
    assert_eq!(out.order, ["set", "set-done"]);
    let (store, rows, truncated) = &out.sets[0];
    assert_eq!((*rows, *truncated), (3, false));

    let kinds: Vec<ColumnKind> = store.status().columns.iter().map(|c| c.kind).collect();
    let json = if json_is_text { ColumnKind::Text } else { ColumnKind::Json };
    assert_eq!(kinds, [ColumnKind::Number, ColumnKind::Text, ColumnKind::Date, json, ColumnKind::Binary, ColumnKind::Text]);

    assert_eq!(cell(store, 0, 1).as_deref(), Some("Ada Lovelace"));
    assert_eq!(cell(store, 0, 2).as_deref(), Some("1815-12-10"));
    assert_eq!(store.cell(0, 4), Some(Some(b"\x89PNG\r\n\x1a\n".to_vec())), "binary bytes arrive untouched");
    assert_eq!(cell(store, 1, 5), None, "NULL stays NULL");
    assert_eq!(cell(store, 2, 1).as_deref(), Some("Zoë Müller"));
    assert_eq!(cell(store, 2, 5).as_deref(), Some("Non-ASCII name: ß, 漢字, 😀"));
}

async fn check_changing_rows(session: &mut Session) {
    let created = run(session, "CREATE TEMPORARY TABLE scratch (id INT AUTO_INCREMENT PRIMARY KEY, v VARCHAR(10))", None).await;
    assert_eq!(created.order, ["done"]);

    let inserted = run(session, "INSERT INTO scratch (v) VALUES ('a'), ('b'), ('c')", None).await;
    assert_eq!(inserted.order, ["done"]);
    assert_eq!(inserted.done[0].0, 3, "affected rows");
    assert_eq!(inserted.done[0].1, Some(1), "first generated id");

    let updated = run(session, "UPDATE scratch SET v = 'z' WHERE id > 1", None).await;
    assert_eq!(updated.done[0].0, 2);
    assert!(updated.done[0].2.contains("Rows matched: 2"), "info was {:?}", updated.done[0].2);

    let selected = run(session, "SELECT v FROM scratch ORDER BY id", None).await;
    let (store, rows, _) = &selected.sets[0];
    assert_eq!(*rows, 3);
    assert_eq!(cell(store, 2, 0).as_deref(), Some("z"));

    let empty = run(session, "SELECT v FROM scratch WHERE id > 100", None).await;
    assert_eq!(empty.order, ["set", "set-done"]);
    assert_eq!(empty.sets[0].1, 0);
    assert_eq!(empty.sets[0].0.status().columns.len(), 1, "an empty result still has its columns");
}

async fn check_row_limit(session: &mut Session, big_query: &str) {
    let started = Instant::now();
    let out = run(session, big_query, Some(1000)).await;
    let (store, rows, truncated) = &out.sets[0];
    assert_eq!((*rows, *truncated), (1000, true));
    assert_eq!(store.status().rows, 1000);
    assert!(started.elapsed() < Duration::from_secs(15), "the server kept running the query for {:?}", started.elapsed());

    // The interrupt must not spill over into the next statements.
    for expected in ["1", "2", "3"] {
        let next = run(session, &format!("SELECT {expected}"), None).await;
        assert_eq!(cell(&next.sets[0].0, 0, 0).as_deref(), Some(expected));
    }
}

fn change(column: usize, value: Option<&str>) -> CellChange {
    CellChange { column, value: value.map(str::to_string) }
}

fn texts(store: &ResultStore) -> Vec<Vec<Option<String>>> {
    let status = store.status();
    (0..status.rows).map(|row| (0..status.columns.len()).map(|column| cell(store, row, column)).collect()).collect()
}

/// Editing rows through the grid: planned statements run in one transaction.
async fn check_editing(session: &mut Session) {
    run(session, "DROP TABLE IF EXISTS sqltool_test.edit_scratch", None).await;
    run(session, "CREATE TABLE sqltool_test.edit_scratch (id INT PRIMARY KEY, name VARCHAR(50) NULL, qty INT NOT NULL DEFAULT 0)", None)
        .await;
    run(session, "INSERT INTO sqltool_test.edit_scratch VALUES (1, 'one', 10), (2, 'two', 20), (3, 'three', 30)", None).await;

    let read = run(session, "SELECT id, name, qty, qty * 2 AS doubled FROM sqltool_test.edit_scratch ORDER BY id", None).await;
    let store = &read.sets[0].0;
    let target = session.edit_target(store).await.expect("editable");
    assert_eq!(target.table, "edit_scratch");
    assert_eq!(target.key_columns, [0]);
    assert_eq!(target.columns[3], None, "a computed column cannot be edited");

    let awkward = "O'Neil \\ ünï; DROP TABLE x --";
    let planned = edit::plan(
        &target,
        store,
        &[
            RowEdit::Update { row: 0, changes: vec![change(1, Some(awkward))] },
            RowEdit::Update { row: 1, changes: vec![change(1, None), change(2, Some("25"))] },
            RowEdit::Delete { row: 2 },
            RowEdit::Insert { values: vec![change(0, Some("4")), change(1, Some("four"))] },
        ],
    )
    .unwrap();
    session.apply(&planned).await.unwrap();

    let after = run(session, "SELECT id, name, qty FROM sqltool_test.edit_scratch ORDER BY id", None).await;
    let s = |v: &str| Some(v.to_string());
    assert_eq!(texts(&after.sets[0].0), [vec![s("1"), s(awkward), s("10")], vec![s("2"), None, s("25")], vec![s("4"), s("four"), s("0")]]);

    // A failure part way through leaves everything as it was: the delete
    // targets row 3, which the batch above already removed.
    let doomed = edit::plan(
        &target,
        store,
        &[RowEdit::Update { row: 0, changes: vec![change(1, Some("should not stick"))] }, RowEdit::Delete { row: 2 }],
    )
    .unwrap();
    assert!(session.apply(&doomed).await.unwrap_err().to_string().contains("no longer exists"));
    let unchanged = run(session, "SELECT name FROM sqltool_test.edit_scratch WHERE id = 1", None).await;
    assert_eq!(cell(&unchanged.sets[0].0, 0, 0).as_deref(), Some(awkward));

    // Results that cannot name a row are not editable.
    let no_key = run(session, "SELECT name FROM sqltool_test.edit_scratch", None).await;
    assert!(session.edit_target(&no_key.sets[0].0).await.is_err());
    let view = run(session, "SELECT id, name FROM sqltool_test.people_names", None).await;
    assert!(session.edit_target(&view.sets[0].0).await.is_err());
    let computed = run(session, "SELECT 1 AS one", None).await;
    assert!(session.edit_target(&computed.sets[0].0).await.is_err());

    run(session, "DROP TABLE sqltool_test.edit_scratch", None).await;
}

fn new_column(name: &str, data_type: &str) -> ColumnSpec {
    ColumnSpec { name: name.into(), data_type: data_type.into(), ..ColumnSpec::default() }
}

/// What must survive a round trip through the server.
fn essentials(spec: &TableSpec) -> Vec<String> {
    let mut out: Vec<String> = spec
        .columns
        .iter()
        .map(|c| {
            format!(
                "{} {} null={} default={:?} on_update={:?} ai={} comment={}",
                c.name,
                c.data_type,
                c.nullable,
                c.default.as_ref().map(|d| d.to_ascii_lowercase()),
                c.on_update.as_ref().map(|d| d.to_ascii_lowercase()),
                c.auto_increment,
                c.comment
            )
        })
        .collect();
    out.push(format!("pk {:?}", spec.primary_key));
    out.extend(spec.indexes.iter().map(|i| format!("index {} unique={} {:?}", i.name, i.unique, i.columns)));
    out.push(format!("comment {}", spec.comment));
    out
}

/// The table designer: create, read back, alter by difference, read back.
async fn check_table_designer(session: &mut Session) {
    for name in ["design_a", "design_b", "design_child"] {
        run(session, &format!("DROP TABLE IF EXISTS sqltool_test.{name}"), None).await;
    }
    let spec = TableSpec {
        schema: "sqltool_test".into(),
        name: "design_a".into(),
        columns: vec![
            ColumnSpec { auto_increment: true, ..new_column("id", "INT") },
            ColumnSpec { default: Some("'untitled'".into()), ..new_column("title", "VARCHAR(100)") },
            ColumnSpec { default: Some("0".into()), ..new_column("qty", "INT") },
            ColumnSpec { nullable: true, ..new_column("note", "TEXT") },
            ColumnSpec {
                default: Some("CURRENT_TIMESTAMP".into()),
                on_update: Some("CURRENT_TIMESTAMP".into()),
                comment: "last change; it's kept".into(),
                ..new_column("changed_at", "TIMESTAMP")
            },
        ],
        primary_key: vec!["id".into()],
        indexes: vec![
            IndexSpec { name: "idx_title".into(), columns: vec!["title".into()], ..IndexSpec::default() },
            IndexSpec { name: "uq_qty_title".into(), columns: vec!["qty".into(), "title".into()], unique: true, ..IndexSpec::default() },
        ],
        foreign_keys: Vec::new(),
        engine: Some("InnoDB".into()),
        collation: None,
        comment: "designer test".into(),
    };
    run(session, &table::create_table(&spec).unwrap(), None).await;

    let read = session.table_spec("sqltool_test", "design_a").await.unwrap().expect("table exists");
    assert_eq!(read.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id", "title", "qty", "note", "changed_at"]);
    assert!(read.columns[0].auto_increment && !read.columns[0].nullable);
    assert_eq!(read.columns[1].default.as_deref(), Some("'untitled'"));
    assert_eq!(read.columns[2].default.as_deref(), Some("0"));
    assert!(read.columns[3].nullable && read.columns[3].default.is_none());
    assert!(read.columns[4].default.as_ref().unwrap().to_ascii_lowercase().starts_with("current_timestamp"));
    assert!(read.columns[4].on_update.as_ref().unwrap().to_ascii_lowercase().starts_with("current_timestamp"));
    assert_eq!(read.columns[4].comment, "last change; it's kept");
    assert_eq!(read.primary_key, ["id"]);
    assert_eq!(read.indexes.len(), 2);
    assert!(read.indexes.iter().any(|i| i.name == "uq_qty_title" && i.unique && i.columns == ["qty", "title"]));
    assert!(read.columns.iter().all(|c| !c.locked) && read.indexes.iter().all(|i| !i.locked));
    assert_eq!(read.comment, "designer test");
    assert!(table::alter_table(&read, &read).is_err(), "nothing changed, so nothing to run");

    // Rename a column, widen one, drop one, add one, add an index, change the comment.
    let mut updated = read.clone();
    updated.columns[1].name = "heading".into();
    updated.columns[2].data_type = "BIGINT".into();
    updated.columns.remove(3);
    updated.columns.insert(1, ColumnSpec { nullable: true, ..new_column("code", "VARCHAR(10)") });
    for index in &mut updated.indexes {
        for column in &mut index.columns {
            if column == "title" {
                *column = "heading".into();
            }
        }
    }
    updated.indexes.push(IndexSpec { name: "idx_code".into(), columns: vec!["code".into()], ..IndexSpec::default() });
    updated.comment = "changed".into();
    let alter = table::alter_table(&read, &updated).unwrap();
    assert!(!alter.contains("DROP INDEX"), "indexes follow the renamed column: {alter}");
    run(session, &alter, None).await;

    let after = session.table_spec("sqltool_test", "design_a").await.unwrap().unwrap();
    assert_eq!(after.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id", "code", "heading", "qty", "changed_at"]);
    assert!(after.columns[3].data_type.to_ascii_lowercase().starts_with("bigint"));
    assert_eq!(after.columns[3].default.as_deref(), Some("0"), "the default survives the type change");
    assert!(after.indexes.iter().any(|i| i.name == "idx_title" && i.columns == ["heading"]));
    assert!(after.indexes.iter().any(|i| i.name == "idx_code"));
    assert_eq!(after.comment, "changed");
    assert!(after.columns[4].on_update.is_some(), "an untouched column keeps its ON UPDATE");

    // What was read back is enough to build an identical table.
    let mut copy = after.clone();
    copy.name = "design_b".into();
    run(session, &table::create_table(&copy).unwrap(), None).await;
    let rebuilt = session.table_spec("sqltool_test", "design_b").await.unwrap().unwrap();
    assert_eq!(essentials(&rebuilt), essentials(&after));

    // Foreign keys are reported for the information pane.
    run(
        session,
        "CREATE TABLE sqltool_test.design_child (id INT PRIMARY KEY, parent INT NOT NULL, \
         CONSTRAINT fk_parent FOREIGN KEY (parent) REFERENCES sqltool_test.design_a (id) ON DELETE CASCADE)",
        None,
    )
    .await;
    let keys = session.foreign_keys("sqltool_test", "design_child").await.unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!((keys[0].name.as_str(), keys[0].referenced_table.as_str()), ("fk_parent", "design_a"));
    assert_eq!((keys[0].columns.as_slice(), keys[0].referenced_columns.as_slice()), (&["parent".to_string()][..], &["id".to_string()][..]));
    assert_eq!(keys[0].on_delete, "CASCADE");
    assert!(session.foreign_keys("sqltool_test", "design_a").await.unwrap().is_empty());

    // The designer reads the key and leaves it alone unless it is changed.
    let child = session.table_spec("sqltool_test", "design_child").await.unwrap().unwrap();
    assert_eq!(child.foreign_keys.len(), 1);
    assert_eq!(child.foreign_keys[0].original_name.as_deref(), Some("fk_parent"));
    assert!(table::alter_table(&child, &child).is_err(), "an untouched key is not rebuilt");

    // Change one key's rule and add a second key on a new column.
    let mut changed = child.clone();
    changed.foreign_keys[0].on_delete = "RESTRICT".into();
    changed.columns.push(ColumnSpec { nullable: true, ..new_column("other", "INT") });
    changed.foreign_keys.push(ForeignKeySpec {
        name: "fk_other".into(),
        columns: vec!["other".into()],
        referenced_table: "design_b".into(),
        referenced_columns: vec!["id".into()],
        on_delete: "SET NULL".into(),
        ..ForeignKeySpec::default()
    });
    let alter = table::alter_table(&child, &changed).unwrap();
    let statements = split::split(&alter);
    assert_eq!(statements.len(), 2, "the changed key is dropped in a statement of its own: {alter}");
    for statement in &statements {
        run(session, &statement.text, None).await;
    }
    let keys = session.foreign_keys("sqltool_test", "design_child").await.unwrap();
    let rule = |name: &str| keys.iter().find(|k| k.name == name).map(|k| (k.referenced_table.as_str(), k.on_delete.as_str()));
    assert_eq!(rule("fk_parent"), Some(("design_a", "RESTRICT")));
    assert_eq!(rule("fk_other"), Some(("design_b", "SET NULL")));

    // What the server reports back is again "nothing to change".
    let reread = session.table_spec("sqltool_test", "design_child").await.unwrap().unwrap();
    assert!(table::alter_table(&reread, &reread).is_err());

    // Removing a column together with its key and the index the server made for it.
    let mut dropped = reread.clone();
    dropped.foreign_keys.retain(|k| k.name != "fk_other");
    dropped.indexes.retain(|i| !i.columns.contains(&"other".to_string()));
    dropped.columns.retain(|c| c.name != "other");
    for statement in split::split(&table::alter_table(&reread, &dropped).unwrap()) {
        run(session, &statement.text, None).await;
    }
    let left = session.table_spec("sqltool_test", "design_child").await.unwrap().unwrap();
    assert_eq!(left.foreign_keys.iter().map(|k| k.name.as_str()).collect::<Vec<_>>(), ["fk_parent"]);
    assert!(left.columns.iter().all(|c| c.name != "other"));

    for name in ["design_child", "design_b", "design_a"] {
        run(session, &table::drop_object(ObjectKind::Table, "sqltool_test", name), None).await;
    }
    assert_eq!(session.table_spec("sqltool_test", "design_a").await.unwrap(), None);
}

/// Completion data, sorting and filtering, and the read-only switch.
async fn check_extras(profile: &ConnectProfile) {
    let mut session = Session::connect(profile).await.unwrap();
    let completion = session.completion_columns("sqltool_test").await.unwrap();
    assert!(completion.contains(&("people".to_string(), "email".to_string())));

    let out = run(&mut session, "SELECT id, name FROM sqltool_test.people", None).await;
    let store = &out.sets[0].0;
    assert_eq!(store.set_view(None, Some((1, true))), 3);
    assert_eq!(cell(store, 0, 1).as_deref(), Some("Zoë Müller"), "sorted by name, descending");
    assert_eq!(store.set_view(Some("grace"), None), 1);
    assert_eq!(cell(store, 0, 0).as_deref(), Some("2"));

    session.set_read_only().await.unwrap();
    assert!(session.run("SELECT COUNT(*) FROM sqltool_test.people", None, |_| {}).await.is_ok());
    let refused =
        session.run("INSERT INTO sqltool_test.people (name) VALUES ('must not appear')", None, |_| {}).await.unwrap_err().to_string();
    assert!(refused.contains("READ ONLY") || refused.contains("read-only") || refused.contains("1792"), "{refused}");
}

/// Exporting a query writes every row it returns, however many, straight to
/// the file. This is the case that went wrong: a result held to the row
/// limit was exported as if it were the whole table.
async fn check_full_export(profile: &ConnectProfile, sql: &str, expected: u64) {
    use sqltool_core::export::ExportFormat;
    use std::io::Write;

    let mut session = Session::connect(profile).await.unwrap();
    let path = std::env::temp_dir().join(format!("sqltool-export-{}-{expected}.csv", std::process::id()));
    let mut writer = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
    let started = Instant::now();
    let mut last = 0;
    let rows = session.export(sql, ExportFormat::Csv, "", &mut writer, |n| last = n).await.unwrap();
    writer.flush().unwrap();
    drop(writer);
    assert_eq!(rows, expected);
    assert_eq!(last, expected);

    // Count the lines on disk rather than trust the number returned.
    use std::io::BufRead;
    let lines = std::io::BufReader::new(std::fs::File::open(&path).unwrap()).lines().count() as u64;
    assert_eq!(lines, expected + 1, "a header line and every row");
    let bytes = std::fs::metadata(&path).unwrap().len();
    println!("exported {expected} rows, {bytes} bytes, in {:?}", started.elapsed());
    let _ = std::fs::remove_file(&path);

    // The connection is usable afterwards.
    assert_eq!(session.query("SELECT 1").await.unwrap().rows[0][0].as_deref(), Some("1"));

    // A statement with no result set is refused, not written as an empty file.
    let mut none: Vec<u8> = Vec::new();
    let refused = session.export("DO 0", ExportFormat::Csv, "", &mut none, |_| {}).await.unwrap_err();
    assert!(refused.to_string().contains("no rows to export"), "{refused}");
}

#[tokio::test]
#[ignore = "needs the MariaDB test server from tests/servers/mariadb-pam.sh"]
async fn mariadb_exports_six_million_rows() {
    check_full_export(&mariadb(), "SELECT seq, CONCAT('row ', seq) FROM seq_1_to_6000000", 6_000_000).await;
}

#[tokio::test]
#[ignore = "needs the MySQL 8.4 test server from tests/servers/mysql84.sh"]
async fn mysql84_exports_a_million_rows() {
    check_full_export(
        &mysql84(),
        "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < 1000000) \
         SELECT /*+ SET_VAR(cte_max_recursion_depth = 1000000) */ n, CONCAT('row ', n) FROM s",
        1_000_000,
    )
    .await;
}

/// A result loaded in full (no row limit), part of it on disk, is exported
/// in full from what is held: every row, without running anything again.
async fn check_loaded_export(profile: &ConnectProfile, sql: &str, expected: u64) {
    use sqltool_core::export::{self, ExportFormat};
    use std::io::{BufRead, Write};

    // Small enough that most of the result goes to the temporary file.
    sqltool_core::store::set_memory_budget(32 * 1024 * 1024);
    let mut session = Session::connect(profile).await.unwrap();
    let out = run(&mut session, sql, None).await;
    let (store, rows, truncated) = &out.sets[0];
    assert_eq!((*rows, *truncated), (expected, false));
    let status = store.status();
    assert_eq!((status.rows, status.total_rows, status.done), (expected, expected, true));
    println!("loaded {expected} rows: {} bytes, {} of them on disk", status.bytes, status.spilled_bytes);

    for format in [ExportFormat::Csv, ExportFormat::Sql] {
        let path = std::env::temp_dir().join(format!("sqltool-loaded-{}-{expected}.{}", std::process::id(), format.extension()));
        let mut writer = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        let written = export::write(store, format, "copy", &mut writer).unwrap();
        writer.flush().unwrap();
        drop(writer);
        assert_eq!(written, expected, "{format:?}: rows reported");
        let lines = std::io::BufReader::new(std::fs::File::open(&path).unwrap()).lines().count() as u64;
        let header = u64::from(format == ExportFormat::Csv);
        assert_eq!(lines, expected + header, "{format:?}: lines in the file");
        let _ = std::fs::remove_file(&path);
    }
    // The last row is what the server sent last.
    assert_eq!(cell(store, expected - 1, 0).as_deref(), Some(expected.to_string().as_str()));
}

#[tokio::test]
#[ignore = "needs the MariaDB test server from tests/servers/mariadb-pam.sh"]
async fn mariadb_exports_a_fully_loaded_six_million_row_result() {
    check_loaded_export(&mariadb(), "SELECT seq, CONCAT('row ', seq), REPEAT('x', 40) FROM seq_1_to_6000000", 6_000_000).await;
}

/// A connection the server has closed is noticed and reopened before its
/// next use, and the handle for cancelling follows it.
async fn check_reconnect(profile: &ConnectProfile) {
    let mut session = Session::connect(profile).await.unwrap();
    let first = session.info().connection_id;
    let canceller = session.canceller();
    assert!(!session.ensure_alive(Duration::ZERO).await.unwrap(), "a live connection is left alone");

    // Another connection kills it, which is what a server's idle timeout amounts to.
    let mut other = Session::connect(profile).await.unwrap();
    other.query(&format!("KILL {first}")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(session.ensure_alive(Duration::ZERO).await.unwrap(), "the dead connection was not noticed");
    let second = session.info().connection_id;
    assert_ne!(first, second, "a new connection has a new id");
    assert_eq!(session.current_database().await.unwrap().as_deref(), Some("sqltool_test"), "the default database is selected again");

    // The handle made before the reconnect interrupts the new connection.
    let started = Instant::now();
    let kill = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        canceller.cancel().await
    });
    let _ = session.run("SELECT SLEEP(20)", None, |_| {}).await;
    assert!(started.elapsed() < Duration::from_secs(10), "cancel after reconnect took {:?}", started.elapsed());
    kill.await.unwrap().unwrap();
    session.absorb_stray_kill().await;

    // A statement that fails because the connection is gone marks the
    // session, and the next use reopens it even though it is not idle.
    other.query(&format!("KILL {second}")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let lost = session.query("SELECT 1").await.unwrap_err();
    assert!(lost.is_connection_lost(), "not seen as a lost connection: {lost}");
    assert!(session.ensure_alive(Duration::from_secs(3600)).await.unwrap(), "a broken session must reconnect");
    assert_eq!(session.query("SELECT 2").await.unwrap().rows[0][0].as_deref(), Some("2"));

    // Read-only mode comes back with the new connection.
    session.set_read_only().await.unwrap();
    let third = session.info().connection_id;
    other.query(&format!("KILL {third}")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(session.ensure_alive(Duration::ZERO).await.unwrap());
    let refused = session.query("CREATE TABLE sqltool_test.should_not_exist (id INT)").await.unwrap_err();
    assert!(!refused.is_connection_lost(), "{refused}");
}

#[tokio::test]
#[ignore = "needs the MariaDB test server from tests/servers/mariadb-pam.sh"]
async fn mariadb_reconnects_after_the_server_drops_the_connection() {
    check_reconnect(&mariadb()).await;
}

#[tokio::test]
#[ignore = "needs the MySQL 8.4 test server from tests/servers/mysql84.sh"]
async fn mysql84_reconnects_after_the_server_drops_the_connection() {
    check_reconnect(&mysql84()).await;
}

#[tokio::test]
#[ignore = "needs the MariaDB test server from tests/servers/mariadb-pam.sh"]
async fn mariadb_editing_designer_and_extras() {
    let mut session = Session::connect(&mariadb()).await.unwrap();
    check_editing(&mut session).await;
    check_table_designer(&mut session).await;
    check_extras(&mariadb()).await;
}

#[tokio::test]
#[ignore = "needs the MySQL 8.4 test server from tests/servers/mysql84.sh"]
async fn mysql84_editing_designer_and_extras() {
    let mut session = Session::connect(&mysql84()).await.unwrap();
    check_editing(&mut session).await;
    check_table_designer(&mut session).await;
    check_extras(&mysql84()).await;
}

#[tokio::test]
#[ignore = "needs the MariaDB test server from tests/servers/mariadb-pam.sh"]
async fn mariadb_metadata_rows_changes_and_limit() {
    let mut session = Session::connect(&mariadb()).await.unwrap();
    assert_eq!(session.current_database().await.unwrap().as_deref(), Some("sqltool_test"));
    check_metadata(&mut session, true).await;
    check_rows_and_kinds(&mut session, true).await;
    check_changing_rows(&mut session).await;
    check_row_limit(&mut session, "SELECT seq, CONCAT('row ', seq) FROM seq_1_to_20000000").await;
}

#[tokio::test]
#[ignore = "needs the MariaDB test server from tests/servers/mariadb-pam.sh"]
async fn mariadb_procedure_returning_two_result_sets() {
    let mut session = Session::connect(&mariadb()).await.unwrap();
    let out = run(&mut session, "CALL sqltool_test.two_sets()", None).await;

    assert_eq!(out.sets.len(), 2, "events were {:?}", out.order);
    let first: Vec<String> = out.sets[0].0.status().columns.into_iter().map(|c| c.name).collect();
    let second: Vec<String> = out.sets[1].0.status().columns.into_iter().map(|c| c.name).collect();
    assert_eq!(first, ["a"]);
    assert_eq!(second, ["b", "c"]);
    assert_eq!(cell(&out.sets[0].0, 0, 0).as_deref(), Some("1"));
    assert_eq!(cell(&out.sets[1].0, 0, 1).as_deref(), Some("3"));

    let after = run(&mut session, "SELECT 5", None).await;
    assert_eq!(cell(&after.sets[0].0, 0, 0).as_deref(), Some("5"));
}

#[tokio::test]
#[ignore = "needs the MySQL 8.4 test server from tests/servers/mysql84.sh"]
async fn mysql84_metadata_rows_changes_and_limit() {
    let mut session = Session::connect(&mysql84()).await.unwrap();
    assert!(session.info().version.at_least(8, 4, 0));
    check_metadata(&mut session, false).await;
    check_rows_and_kinds(&mut session, false).await;
    check_changing_rows(&mut session).await;
    check_row_limit(
        &mut session,
        "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < 20000000) \
         SELECT /*+ SET_VAR(cte_max_recursion_depth = 20000000) */ n, CONCAT('row ', n) FROM s",
    )
    .await;
}
