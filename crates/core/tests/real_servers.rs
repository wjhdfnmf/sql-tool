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

use sqltool_core::{ddl, CaSource, ColumnKind, ConnectProfile, ObjectKind, ResultStore, RunEvent, Session};

fn local_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/servers/.local")
}

fn env_file(name: &str) -> HashMap<String, String> {
    let path = local_dir().join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} is missing ({e}); run the scripts in tests/servers first", path.display()));
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
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
            RunEvent::Done {
                affected_rows,
                last_insert_id,
                info,
                ..
            } => {
                out.order.push("done");
                out.done.push((affected_rows, last_insert_id, info));
            }
        })
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
    out
}

fn cell(store: &ResultStore, row: u64, column: usize) -> Option<String> {
    store
        .cell(row, column)
        .expect("cell exists")
        .map(|bytes| String::from_utf8(bytes).unwrap())
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
    let out = run(
        session,
        "SELECT id, name, born, profile, avatar, notes FROM sqltool_test.people ORDER BY id",
        None,
    )
    .await;
    assert_eq!(out.order, ["set", "set-done"]);
    let (store, rows, truncated) = &out.sets[0];
    assert_eq!((*rows, *truncated), (3, false));

    let kinds: Vec<ColumnKind> = store.status().columns.iter().map(|c| c.kind).collect();
    let json = if json_is_text { ColumnKind::Text } else { ColumnKind::Json };
    assert_eq!(
        kinds,
        [ColumnKind::Number, ColumnKind::Text, ColumnKind::Date, json, ColumnKind::Binary, ColumnKind::Text]
    );

    assert_eq!(cell(store, 0, 1).as_deref(), Some("Ada Lovelace"));
    assert_eq!(cell(store, 0, 2).as_deref(), Some("1815-12-10"));
    assert_eq!(store.cell(0, 4), Some(Some(b"\x89PNG\r\n\x1a\n".to_vec())), "binary bytes arrive untouched");
    assert_eq!(cell(store, 1, 5), None, "NULL stays NULL");
    assert_eq!(cell(store, 2, 1).as_deref(), Some("Zoë Müller"));
    assert_eq!(cell(store, 2, 5).as_deref(), Some("Non-ASCII name: ß, 漢字, 😀"));
}

async fn check_changing_rows(session: &mut Session) {
    let created = run(
        session,
        "CREATE TEMPORARY TABLE scratch (id INT AUTO_INCREMENT PRIMARY KEY, v VARCHAR(10))",
        None,
    )
    .await;
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
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "the server kept running the query for {:?}",
        started.elapsed()
    );

    // The interrupt must not spill over into the next statements.
    for expected in ["1", "2", "3"] {
        let next = run(session, &format!("SELECT {expected}"), None).await;
        assert_eq!(cell(&next.sets[0].0, 0, 0).as_deref(), Some(expected));
    }
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
