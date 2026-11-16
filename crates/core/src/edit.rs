//! Turning edits made in the result grid into SQL.
//!
//! A result can be edited when every column comes from one table and the
//! result includes that table's whole primary key. Each change becomes one
//! statement that names the row by its key, using the values the row had
//! when it was read. Statements run with the values sent separately from the
//! SQL; the text shown to the user for review is the same statement with the
//! values written in.

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::export::sql_literal;
use crate::split::quote_identifier;
use crate::store::{ColumnInfo, ColumnKind, ResultStore};

/// Where a result's edits go.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditTarget {
    pub schema: String,
    pub table: String,
    /// Result columns that make up the table's primary key.
    pub key_columns: Vec<usize>,
    /// For each result column, the table column it maps to, if it can be edited.
    pub columns: Vec<Option<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellChange {
    /// Result column index.
    pub column: usize,
    /// `None` sets the cell to NULL.
    pub value: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RowEdit {
    Update { row: u64, changes: Vec<CellChange> },
    Delete { row: u64 },
    Insert { values: Vec<CellChange> },
}

/// One statement ready to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedStatement {
    /// With `?` placeholders.
    pub sql: String,
    pub params: Vec<Option<Vec<u8>>>,
    /// With the values written in, for the user to review.
    pub preview: String,
    /// The statement must match exactly one existing row.
    pub must_match_one_row: bool,
}

/// Works out whether the result can be edited, from its columns alone.
/// `primary_key_size` is the number of columns in the table's primary key,
/// which the caller looks up once the table is known.
pub fn edit_target(columns: &[ColumnInfo]) -> Result<EditTarget, String> {
    let mut table: Option<(&str, &str)> = None;
    for column in columns {
        if let Some(source) = &column.source {
            let this = (source.schema.as_str(), source.table.as_str());
            match table {
                None => table = Some(this),
                Some(seen) if seen != this => {
                    return Err("The result combines more than one table.".into());
                }
                _ => {}
            }
        }
    }
    let Some((schema, name)) = table else {
        return Err("The result does not come straight from a table.".into());
    };
    let key_columns: Vec<usize> = columns
        .iter()
        .enumerate()
        .filter(|(_, c)| c.source.is_some() && c.primary_key)
        .map(|(i, _)| i)
        .collect();
    if key_columns.is_empty() {
        return Err("The result does not include the table's primary key.".into());
    }
    Ok(EditTarget {
        schema: schema.to_string(),
        table: name.to_string(),
        key_columns,
        columns: columns
            .iter()
            // Binary values cannot be typed into a text box.
            .map(|c| c.source.as_ref().filter(|_| c.kind != ColumnKind::Binary).map(|s| s.column.clone()))
            .collect(),
    })
}

fn fail(message: impl Into<String>) -> Error {
    Error::Other(message.into())
}

/// Builds the statements for `edits`, reading key values from `store`.
pub fn plan(target: &EditTarget, store: &ResultStore, edits: &[RowEdit]) -> Result<Vec<PlannedStatement>, Error> {
    let info = store.status().columns;
    let table = format!("{}.{}", quote_identifier(&target.schema), quote_identifier(&target.table));
    let kind = |column: usize| info.get(column).map_or(ColumnKind::Text, |c| c.kind);
    let column_name = |column: usize| -> Result<&str, Error> {
        target
            .columns
            .get(column)
            .and_then(Option::as_deref)
            .ok_or_else(|| fail("That column cannot be edited."))
    };

    // `WHERE key = ? AND ...` for a row, from the values it had when read.
    let locate = |row: u64| -> Result<(String, String, Vec<Option<Vec<u8>>>), Error> {
        let cells = store.row(row).ok_or_else(|| fail("That row is no longer in the result."))?;
        let mut sql = Vec::new();
        let mut preview = Vec::new();
        let mut params = Vec::new();
        for &key in &target.key_columns {
            let name = quote_identifier(&info[key].source.as_ref().expect("key columns have a source").column);
            let value = cells.get(key).cloned().flatten();
            let Some(value) = value else {
                return Err(fail("A row whose key is NULL cannot be identified."));
            };
            sql.push(format!("{name} = ?"));
            preview.push(format!("{name} = {}", sql_literal(Some(&value), kind(key))));
            params.push(Some(value));
        }
        Ok((sql.join(" AND "), preview.join(" AND "), params))
    };

    let assignments = |changes: &[CellChange]| -> Result<(Vec<String>, Vec<String>, Vec<Option<Vec<u8>>>), Error> {
        if changes.is_empty() {
            return Err(fail("There is nothing to change."));
        }
        let mut names = Vec::new();
        let mut literals = Vec::new();
        let mut params = Vec::new();
        for change in changes {
            names.push(quote_identifier(column_name(change.column)?));
            let bytes = change.value.as_ref().map(|v| v.as_bytes().to_vec());
            literals.push(sql_literal(bytes.as_deref(), kind(change.column)));
            params.push(bytes);
        }
        Ok((names, literals, params))
    };

    edits
        .iter()
        .map(|edit| match edit {
            RowEdit::Update { row, changes } => {
                let (names, literals, mut params) = assignments(changes)?;
                let (where_sql, where_preview, key_params) = locate(*row)?;
                params.extend(key_params);
                let set_sql: Vec<String> = names.iter().map(|n| format!("{n} = ?")).collect();
                let set_preview: Vec<String> = names.iter().zip(&literals).map(|(n, l)| format!("{n} = {l}")).collect();
                Ok(PlannedStatement {
                    sql: format!("UPDATE {table} SET {} WHERE {where_sql} LIMIT 1", set_sql.join(", ")),
                    preview: format!("UPDATE {table} SET {} WHERE {where_preview} LIMIT 1;", set_preview.join(", ")),
                    params,
                    must_match_one_row: false,
                })
            }
            RowEdit::Delete { row } => {
                let (where_sql, where_preview, params) = locate(*row)?;
                Ok(PlannedStatement {
                    sql: format!("DELETE FROM {table} WHERE {where_sql} LIMIT 1"),
                    preview: format!("DELETE FROM {table} WHERE {where_preview} LIMIT 1;"),
                    params,
                    must_match_one_row: true,
                })
            }
            RowEdit::Insert { values } => {
                let (names, literals, params) = assignments(values)?;
                let marks = vec!["?"; names.len()].join(", ");
                Ok(PlannedStatement {
                    sql: format!("INSERT INTO {table} ({}) VALUES ({marks})", names.join(", ")),
                    preview: format!("INSERT INTO {table} ({}) VALUES ({});", names.join(", "), literals.join(", ")),
                    params,
                    must_match_one_row: false,
                })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ColumnSource, RowBatch};

    fn from_table(name: &str, kind: ColumnKind, table: &str, primary_key: bool) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            kind,
            source: Some(ColumnSource {
                schema: "shop".into(),
                table: table.into(),
                column: name.to_string(),
            }),
            primary_key,
        }
    }

    fn people() -> (Vec<ColumnInfo>, ResultStore) {
        let columns = vec![
            from_table("id", ColumnKind::Number, "people", true),
            from_table("name", ColumnKind::Text, "people", false),
            from_table("photo", ColumnKind::Binary, "people", false),
            ColumnInfo::plain("UPPER(name)", ColumnKind::Text),
        ];
        let store = ResultStore::new();
        store.set_columns(columns.clone());
        let mut batch = RowBatch::default();
        batch.push_row([Some(&b"7"[..]), Some(&b"Ann"[..]), None, Some(&b"ANN"[..])]);
        batch.push_row([Some(&b"8"[..]), Some(&b"O'Brien"[..]), None, Some(&b"O'BRIEN"[..])]);
        store.append(&mut batch);
        (columns, store)
    }

    #[test]
    fn a_single_table_result_with_its_key_is_editable() {
        let (columns, _) = people();
        let target = edit_target(&columns).unwrap();
        assert_eq!((target.schema.as_str(), target.table.as_str()), ("shop", "people"));
        assert_eq!(target.key_columns, [0]);
        assert_eq!(
            target.columns,
            [Some("id".to_string()), Some("name".to_string()), None, None],
            "binary and computed columns are not editable"
        );
    }

    #[test]
    fn results_without_a_table_a_key_or_from_two_tables_are_not_editable() {
        assert!(edit_target(&[ColumnInfo::plain("1", ColumnKind::Number)]).unwrap_err().contains("straight from a table"));
        assert!(edit_target(&[from_table("name", ColumnKind::Text, "people", false)]).unwrap_err().contains("primary key"));
        let mixed = [from_table("id", ColumnKind::Number, "people", true), from_table("id", ColumnKind::Number, "orders", true)];
        assert!(edit_target(&mixed).unwrap_err().contains("more than one table"));
    }

    #[test]
    fn update_names_the_row_by_its_original_key() {
        let (columns, store) = people();
        let target = edit_target(&columns).unwrap();
        let planned = plan(
            &target,
            &store,
            &[RowEdit::Update {
                row: 1,
                changes: vec![CellChange { column: 1, value: Some("O'Neil".into()) }],
            }],
        )
        .unwrap();
        assert_eq!(planned[0].sql, "UPDATE `shop`.`people` SET `name` = ? WHERE `id` = ? LIMIT 1");
        assert_eq!(planned[0].params, [Some(b"O'Neil".to_vec()), Some(b"8".to_vec())]);
        assert_eq!(planned[0].preview, "UPDATE `shop`.`people` SET `name` = 'O''Neil' WHERE `id` = 8 LIMIT 1;");
    }

    #[test]
    fn null_delete_and_insert_are_planned() {
        let (columns, store) = people();
        let target = edit_target(&columns).unwrap();
        let planned = plan(
            &target,
            &store,
            &[
                RowEdit::Update { row: 0, changes: vec![CellChange { column: 1, value: None }] },
                RowEdit::Delete { row: 0 },
                RowEdit::Insert {
                    values: vec![CellChange { column: 0, value: Some("9".into()) }, CellChange { column: 1, value: Some("Zoë".into()) }],
                },
            ],
        )
        .unwrap();
        assert_eq!(planned[0].preview, "UPDATE `shop`.`people` SET `name` = NULL WHERE `id` = 7 LIMIT 1;");
        assert_eq!(planned[0].params[0], None);
        assert_eq!(planned[1].preview, "DELETE FROM `shop`.`people` WHERE `id` = 7 LIMIT 1;");
        assert!(planned[1].must_match_one_row);
        assert_eq!(planned[2].sql, "INSERT INTO `shop`.`people` (`id`, `name`) VALUES (?, ?)");
        assert_eq!(planned[2].preview, "INSERT INTO `shop`.`people` (`id`, `name`) VALUES (9, 'Zoë');");
    }

    #[test]
    fn computed_binary_and_missing_targets_are_refused() {
        let (columns, store) = people();
        let target = edit_target(&columns).unwrap();
        let change = |column| RowEdit::Update { row: 0, changes: vec![CellChange { column, value: Some("x".into()) }] };
        assert!(plan(&target, &store, &[change(3)]).is_err(), "computed column");
        assert!(plan(&target, &store, &[change(2)]).is_err(), "binary column");
        assert!(plan(&target, &store, &[RowEdit::Delete { row: 99 }]).is_err(), "no such row");
        assert!(plan(&target, &store, &[RowEdit::Update { row: 0, changes: vec![] }]).is_err(), "nothing to change");
    }

    #[test]
    fn rows_are_located_through_the_current_sort() {
        let (columns, store) = people();
        let target = edit_target(&columns).unwrap();
        store.set_view(None, Some((0, true)));
        let planned = plan(&target, &store, &[RowEdit::Delete { row: 0 }]).unwrap();
        assert_eq!(planned[0].params, [Some(b"8".to_vec())], "row 0 is now the highest id");
    }
}

impl crate::session::Session {
    /// Checks that `store` can be edited and says where the edits would go.
    /// The result must include every column of the table's primary key.
    pub async fn edit_target(&mut self, store: &ResultStore) -> Result<EditTarget, Error> {
        let target = edit_target(&store.status().columns).map_err(fail)?;
        let details = self.table_details(&target.schema, &target.table).await?;
        if details.is_none_or(|d| d.is_view) {
            return Err(fail("Views cannot be edited here."));
        }
        let key_size = self
            .table_columns(&target.schema, &target.table)
            .await?
            .iter()
            .filter(|c| c.key == "PRI")
            .count();
        if key_size != target.key_columns.len() {
            return Err(fail("The result does not include every column of the table's primary key."));
        }
        Ok(target)
    }

    /// Applies planned statements in one transaction. If any statement fails,
    /// or a delete does not find its row, nothing is changed.
    pub async fn apply(&mut self, statements: &[PlannedStatement]) -> Result<(), Error> {
        // Transaction control goes over the plain protocol: MySQL does not
        // accept it as a prepared statement.
        self.query("START TRANSACTION").await?;
        for statement in statements {
            let outcome = match self.exec_params(&statement.sql, statement.params.clone()).await {
                Ok(1) => Ok(()),
                Ok(0) if !statement.must_match_one_row => Ok(()),
                Ok(0) => Err(fail("A row to delete no longer exists. Nothing was changed.")),
                Ok(n) => Err(fail(format!("A statement meant for one row matched {n}. Nothing was changed."))),
                Err(e) => Err(e),
            };
            if let Err(e) = outcome {
                let _ = self.query("ROLLBACK").await;
                return Err(e);
            }
        }
        self.query("COMMIT").await?;
        Ok(())
    }
}
