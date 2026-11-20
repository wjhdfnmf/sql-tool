//! Working out what a statement would do before it is sent, for read-only
//! connections and for confirmations on production connections.
//!
//! This looks at keywords only; it does not parse SQL. Where it cannot tell,
//! it assumes the statement changes something.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatementKind {
    /// Only reads: SELECT, SHOW, EXPLAIN, ...
    Read,
    /// Session housekeeping: USE, SET of a session variable, transactions.
    Session,
    /// Changes rows: INSERT, UPDATE, DELETE, ...
    Write,
    /// Changes structure, accounts or server settings: CREATE, DROP, GRANT, SET GLOBAL, ...
    Structure,
    /// Anything else. Treated as a change.
    Other,
}

/// The statement's words outside strings, comments and parentheses, in
/// upper case. Enough to recognise the statement and find a top-level WHERE.
pub fn top_level_words(statement: &str) -> Vec<String> {
    let bytes = statement.as_bytes();
    let mut words = Vec::new();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'\'' | b'"' | b'`' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' && c != b'`' {
                        i += 2;
                    } else if bytes[i] == c {
                        if bytes.get(i + 1) == Some(&c) {
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            b'#' => i = bytes[i..].iter().position(|b| *b == b'\n').map_or(bytes.len(), |p| i + p),
            b'-' if bytes.get(i + 1) == Some(&b'-') && bytes.get(i + 2).is_none_or(|b| b.is_ascii_whitespace()) => {
                i = bytes[i..].iter().position(|b| *b == b'\n').map_or(bytes.len(), |p| i + p);
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = bytes[i + 2..].windows(2).position(|w| w == b"*/").map_or(bytes.len(), |p| i + 2 + p + 2);
            }
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                if depth == 0 {
                    words.push(statement[start..i].to_ascii_uppercase());
                }
            }
            _ => i += 1,
        }
    }
    words
}

pub fn classify(statement: &str) -> StatementKind {
    // A parenthesised query, such as `(SELECT 1) UNION (SELECT 2)`, is
    // judged by what is inside its first parenthesis.
    if let Some(inner) = statement.trim_start().strip_prefix('(') {
        return classify(inner);
    }
    let words = top_level_words(statement);
    let Some(first) = words.first().map(String::as_str) else {
        return StatementKind::Other;
    };
    let has = |word: &str| words.iter().any(|w| w == word);
    match first {
        "SELECT" | "TABLE" | "VALUES" => {
            // SELECT ... INTO OUTFILE writes a file on the server.
            if has("OUTFILE") || has("DUMPFILE") {
                StatementKind::Other
            } else {
                StatementKind::Read
            }
        }
        "SHOW" | "DESCRIBE" | "DESC" | "HELP" => StatementKind::Read,
        // EXPLAIN ANALYZE runs the statement it explains.
        "EXPLAIN" => {
            if words.get(1).is_some_and(|w| w == "ANALYZE") && !has("SELECT") {
                StatementKind::Other
            } else if has("UPDATE") || has("DELETE") || has("INSERT") || has("REPLACE") {
                if has("ANALYZE") {
                    StatementKind::Write
                } else {
                    StatementKind::Read
                }
            } else {
                StatementKind::Read
            }
        }
        // A common table expression can lead into a change in MySQL 8.
        "WITH" => {
            if has("UPDATE") || has("DELETE") || has("INSERT") || has("REPLACE") {
                StatementKind::Write
            } else {
                StatementKind::Read
            }
        }
        "USE" | "START" | "BEGIN" | "COMMIT" | "ROLLBACK" | "SAVEPOINT" | "RELEASE" => StatementKind::Session,
        "SET" => match words.get(1).map(String::as_str) {
            Some("GLOBAL" | "PERSIST" | "PERSIST_ONLY" | "PASSWORD" | "DEFAULT" | "ROLE") => StatementKind::Structure,
            _ => StatementKind::Session,
        },
        "INSERT" | "UPDATE" | "DELETE" | "REPLACE" | "LOAD" | "CALL" | "IMPORT" => StatementKind::Write,
        "CREATE" | "ALTER" | "DROP" | "TRUNCATE" | "RENAME" | "GRANT" | "REVOKE" => StatementKind::Structure,
        _ => StatementKind::Other,
    }
}

/// Whether a read-only connection may run this statement.
pub fn allowed_when_read_only(statement: &str) -> bool {
    matches!(classify(statement), StatementKind::Read | StatementKind::Session)
}

/// Something about a statement worth confirming on a production connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Concern {
    /// Position of the statement among those about to run.
    pub index: usize,
    pub reason: String,
    /// The start of the statement, for display.
    pub sql: String,
}

fn preview(sql: &str) -> String {
    let flat: String = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = flat.chars().take(160).collect();
    if out.len() < flat.len() {
        out.push('\u{2026}');
    }
    out
}

/// Statements that change structure, or change rows without a WHERE clause.
pub fn review<'a>(statements: impl IntoIterator<Item = &'a str>) -> Vec<Concern> {
    let mut concerns = Vec::new();
    for (index, statement) in statements.into_iter().enumerate() {
        let words = top_level_words(statement);
        let first = words.first().map(String::as_str).unwrap_or("");
        let reason = match classify(statement) {
            StatementKind::Structure => Some(format!("{first} changes structure, accounts or server settings")),
            StatementKind::Write if matches!(first, "UPDATE" | "DELETE") && !words.iter().any(|w| w == "WHERE") => {
                Some(format!("{first} without a WHERE clause affects every row"))
            }
            _ => None,
        };
        if let Some(reason) = reason {
            concerns.push(Concern { index, reason, sql: preview(statement) });
        }
    }
    concerns
}

#[cfg(test)]
mod tests {
    use super::*;
    use StatementKind::*;

    #[test]
    fn reads_are_recognised() {
        for sql in [
            "SELECT * FROM t",
            "  select 1",
            "/* hint */ SELECT 1",
            "-- note\nSHOW TABLES",
            "(SELECT 1) UNION (SELECT 2)",
            "EXPLAIN SELECT * FROM t",
            "EXPLAIN UPDATE t SET a = 1",
            "DESCRIBE t",
            "WITH c AS (SELECT 1) SELECT * FROM c",
            "TABLE t",
        ] {
            assert_eq!(classify(sql), Read, "{sql}");
        }
    }

    #[test]
    fn changes_are_recognised() {
        assert_eq!(classify("INSERT INTO t VALUES (1)"), Write);
        assert_eq!(classify("update t set a = 1"), Write);
        assert_eq!(classify("DELETE FROM t"), Write);
        assert_eq!(classify("CALL p()"), Write);
        assert_eq!(classify("WITH c AS (SELECT 1) DELETE FROM t WHERE id IN (SELECT * FROM c)"), Write);
        assert_eq!(classify("CREATE TABLE t (id INT)"), Structure);
        assert_eq!(classify("TRUNCATE t"), Structure);
        assert_eq!(classify("GRANT ALL ON *.* TO a"), Structure);
        assert_eq!(classify("SET GLOBAL max_connections = 5"), Structure);
        assert_eq!(classify("SET PASSWORD = 'x'"), Structure);
        assert_eq!(classify("FLUSH PRIVILEGES"), Other);
        assert_eq!(classify("SELECT * FROM t INTO OUTFILE '/tmp/x'"), Other);
        assert_eq!(classify("EXPLAIN ANALYZE UPDATE t SET a = 1"), Other);
        assert_eq!(classify("ANALYZE UPDATE t SET a = 1"), Other);
    }

    #[test]
    fn session_statements_are_recognised() {
        for sql in ["USE shop", "SET @x = 1", "SET NAMES utf8mb4", "START TRANSACTION", "COMMIT", "ROLLBACK"] {
            assert_eq!(classify(sql), Session, "{sql}");
        }
    }

    #[test]
    fn keywords_inside_strings_comments_and_subqueries_do_not_count() {
        assert_eq!(classify("SELECT 'DROP TABLE t', `delete`, \"update\" FROM t"), Read);
        assert_eq!(classify("SELECT * FROM t /* DELETE */ -- UPDATE\n"), Read);
        assert_eq!(classify("WITH c AS (SELECT 'x') SELECT (SELECT COUNT(*) FROM u WHERE u.deleted = 1) FROM c"), Read);
    }

    #[test]
    fn read_only_allows_reads_and_session_statements_only() {
        assert!(allowed_when_read_only("SELECT 1"));
        assert!(allowed_when_read_only("USE shop"));
        assert!(!allowed_when_read_only("UPDATE t SET a = 1 WHERE id = 2"));
        assert!(!allowed_when_read_only("DROP TABLE t"));
        assert!(!allowed_when_read_only("KILL 5"));
        assert!(!allowed_when_read_only(""));
    }

    #[test]
    fn review_flags_structure_changes_and_missing_where() {
        let concerns = review([
            "SELECT 1",
            "UPDATE t SET a = 1",
            "UPDATE t SET a = 1 WHERE id = 2",
            "DELETE FROM t WHERE id IN (SELECT id FROM u)",
            "delete from t",
            "DROP TABLE t",
            "INSERT INTO t VALUES (1)",
        ]);
        let flagged: Vec<usize> = concerns.iter().map(|c| c.index).collect();
        assert_eq!(flagged, [1, 4, 5]);
        assert!(concerns[0].reason.contains("without a WHERE"));
        assert!(concerns[2].reason.starts_with("DROP"));
    }

    #[test]
    fn a_where_inside_a_subquery_or_string_is_not_a_where_clause() {
        assert_eq!(review(["UPDATE t SET a = (SELECT b FROM u WHERE u.id = 1)"]).len(), 1);
        assert_eq!(review(["DELETE FROM t /* WHERE id = 1 */"]).len(), 1);
        assert_eq!(review(["UPDATE t SET note = 'WHERE x' "]).len(), 1);
    }
}
