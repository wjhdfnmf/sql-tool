//! Reading what exists on the server: schemas, tables, columns, routines.
//!
//! Every query is limited to one schema. Unfiltered `information_schema`
//! queries are slow on large MySQL 5.7 servers, so nothing here scans it
//! whole. Names are passed as parameters, never spliced into the SQL.

use serde::Serialize;

use crate::ddl::ObjectKind;
use crate::error::Error;
use crate::session::Session;
use crate::split::quote_identifier;
use crate::table::{ColumnSpec, IndexSpec, TableSpec};
use crate::version::Flavor;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableEntry {
    pub name: String,
    /// True for views (including system views).
    pub is_view: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineEntry {
    pub name: String,
    /// `PROCEDURE` or `FUNCTION`.
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerEntry {
    pub name: String,
    pub table: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaObjects {
    pub tables: Vec<TableEntry>,
    pub routines: Vec<RoutineEntry>,
    pub triggers: Vec<TriggerEntry>,
    pub events: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnEntry {
    pub name: String,
    /// Full type as the server prints it, for example `varchar(255)`.
    pub column_type: String,
    pub nullable: bool,
    /// `PRI`, `UNI`, `MUL` or empty.
    pub key: String,
    pub default: Option<String>,
    /// For example `auto_increment`.
    pub extra: String,
}

/// Facts about one table or view, for the information pane.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableDetails {
    pub is_view: bool,
    pub engine: Option<String>,
    /// The storage engine's estimate; not an exact count.
    pub rows_estimate: Option<u64>,
    pub collation: Option<String>,
    pub comment: String,
    pub created: Option<String>,
    pub data_bytes: Option<u64>,
    pub index_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaDefaults {
    pub charset: String,
    pub collation: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Charset {
    pub name: String,
    pub default_collation: String,
}

fn number(row: &mut [Option<String>], index: usize) -> Option<u64> {
    row.get_mut(index).and_then(Option::take).and_then(|v| v.parse().ok())
}

fn cell(row: &mut [Option<String>], index: usize) -> String {
    row.get_mut(index).and_then(Option::take).unwrap_or_default()
}

impl Session {
    pub async fn schemas(&mut self) -> Result<Vec<String>, Error> {
        let result = self.query("SHOW DATABASES").await?;
        Ok(result.rows.into_iter().filter_map(|mut row| row.swap_remove(0)).collect())
    }

    pub async fn schema_objects(&mut self, schema: &str) -> Result<SchemaObjects, Error> {
        let tables = self
            .query_params(
                "SELECT table_name, table_type FROM information_schema.tables \
                 WHERE table_schema = ? ORDER BY table_name",
                (schema,),
            )
            .await?
            .into_iter()
            .map(|mut row| TableEntry {
                name: cell(&mut row, 0),
                is_view: cell(&mut row, 1).to_ascii_uppercase().contains("VIEW"),
            })
            .collect();

        let routines = self
            .query_params(
                "SELECT routine_name, routine_type FROM information_schema.routines \
                 WHERE routine_schema = ? ORDER BY routine_name",
                (schema,),
            )
            .await?
            .into_iter()
            .map(|mut row| RoutineEntry {
                name: cell(&mut row, 0),
                kind: cell(&mut row, 1).to_ascii_uppercase(),
            })
            .collect();

        let triggers = self
            .query_params(
                "SELECT trigger_name, event_object_table FROM information_schema.triggers \
                 WHERE trigger_schema = ? ORDER BY trigger_name",
                (schema,),
            )
            .await?
            .into_iter()
            .map(|mut row| TriggerEntry {
                name: cell(&mut row, 0),
                table: cell(&mut row, 1),
            })
            .collect();

        let events = self
            .query_params(
                "SELECT event_name FROM information_schema.events \
                 WHERE event_schema = ? ORDER BY event_name",
                (schema,),
            )
            .await?
            .into_iter()
            .map(|mut row| cell(&mut row, 0))
            .collect();

        Ok(SchemaObjects {
            tables,
            routines,
            triggers,
            events,
        })
    }

    pub async fn table_details(&mut self, schema: &str, table: &str) -> Result<Option<TableDetails>, Error> {
        let rows = self
            .query_params(
                "SELECT table_type, engine, table_rows, table_collation, table_comment, create_time, \
                 data_length, index_length FROM information_schema.tables \
                 WHERE table_schema = ? AND table_name = ?",
                (schema, table),
            )
            .await?;
        Ok(rows.into_iter().next().map(|mut row| TableDetails {
            is_view: cell(&mut row, 0).to_ascii_uppercase().contains("VIEW"),
            engine: row.get_mut(1).and_then(Option::take),
            rows_estimate: number(&mut row, 2),
            collation: row.get_mut(3).and_then(Option::take),
            comment: cell(&mut row, 4),
            created: row.get_mut(5).and_then(Option::take),
            data_bytes: number(&mut row, 6),
            index_bytes: number(&mut row, 7),
        }))
    }

    pub async fn schema_defaults(&mut self, schema: &str) -> Result<Option<SchemaDefaults>, Error> {
        let rows = self
            .query_params(
                "SELECT default_character_set_name, default_collation_name \
                 FROM information_schema.schemata WHERE schema_name = ?",
                (schema,),
            )
            .await?;
        Ok(rows.into_iter().next().map(|mut row| SchemaDefaults {
            charset: cell(&mut row, 0),
            collation: cell(&mut row, 1),
        }))
    }

    /// Character sets this server supports, with each one's default collation.
    pub async fn charsets(&mut self) -> Result<Vec<Charset>, Error> {
        let result = self
            .query(
                "SELECT character_set_name, default_collate_name \
                 FROM information_schema.character_sets ORDER BY character_set_name",
            )
            .await?;
        Ok(result
            .rows
            .into_iter()
            .map(|mut row| Charset {
                name: cell(&mut row, 0),
                default_collation: cell(&mut row, 1),
            })
            .collect())
    }

    pub async fn collations(&mut self, charset: &str) -> Result<Vec<String>, Error> {
        // MariaDB 10.10 added collations that apply to several character
        // sets (the uca1400 family, which includes utf8mb4's default). Those
        // have no character set of their own in `collations`; the full names
        // per character set are in the applicability table instead.
        let version = &self.info().version;
        let sql = if version.flavor == Flavor::MariaDb && version.at_least(10, 10, 0) {
            "SELECT full_collation_name FROM information_schema.collation_character_set_applicability \
             WHERE character_set_name = ? ORDER BY full_collation_name"
        } else {
            "SELECT collation_name FROM information_schema.collations \
             WHERE character_set_name = ? ORDER BY collation_name"
        };
        Ok(self
            .query_params(sql, (charset,))
            .await?
            .into_iter()
            .map(|mut row| cell(&mut row, 0))
            .collect())
    }

    /// The statement that would recreate the object, as the server reports it.
    pub async fn show_create(&mut self, kind: ObjectKind, schema: &str, name: &str) -> Result<String, Error> {
        let sql = format!(
            "SHOW CREATE {} {}.{}",
            kind.keyword(),
            quote_identifier(schema),
            quote_identifier(name)
        );
        // Which column holds the statement depends on the kind of object.
        let column = match kind {
            ObjectKind::Table | ObjectKind::View => 1,
            ObjectKind::Procedure | ObjectKind::Function | ObjectKind::Trigger => 2,
            ObjectKind::Event => 3,
        };
        let mut result = self.query(&sql).await?;
        result
            .rows
            .first_mut()
            .and_then(|row| row.get_mut(column))
            .and_then(Option::take)
            .ok_or_else(|| {
                Error::Other(format!(
                    "The server did not return the definition of {name}. This account may not be allowed to see it."
                ))
            })
    }

    pub async fn table_columns(&mut self, schema: &str, table: &str) -> Result<Vec<ColumnEntry>, Error> {
        Ok(self
            .query_params(
                "SELECT column_name, column_type, is_nullable, column_key, column_default, extra \
                 FROM information_schema.columns \
                 WHERE table_schema = ? AND table_name = ? ORDER BY ordinal_position",
                (schema, table),
            )
            .await?
            .into_iter()
            .map(|mut row| ColumnEntry {
                name: cell(&mut row, 0),
                column_type: cell(&mut row, 1),
                nullable: cell(&mut row, 2).eq_ignore_ascii_case("YES"),
                key: cell(&mut row, 3),
                default: row.get_mut(4).and_then(Option::take),
                extra: cell(&mut row, 5),
            })
            .collect())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKeyEntry {
    pub name: String,
    pub columns: Vec<String>,
    pub referenced_schema: String,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
    pub on_update: String,
    pub on_delete: String,
}

/// The MySQL column types whose defaults are written without quotes.
fn is_numeric_type(column_type: &str) -> bool {
    let base = column_type.split(['(', ' ']).next().unwrap_or("").to_ascii_lowercase();
    matches!(
        base.as_str(),
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" | "decimal" | "numeric" | "float" | "double" | "real" | "bit" | "year"
    )
}

impl Session {
    /// Table and column names of one schema, for editor completion.
    pub async fn completion_columns(&mut self, schema: &str) -> Result<Vec<(String, String)>, Error> {
        Ok(self
            .query_params(
                "SELECT table_name, column_name FROM information_schema.columns \
                 WHERE table_schema = ? ORDER BY table_name, ordinal_position",
                (schema,),
            )
            .await?
            .into_iter()
            .map(|mut row| (cell(&mut row, 0), cell(&mut row, 1)))
            .collect())
    }

    pub async fn foreign_keys(&mut self, schema: &str, table: &str) -> Result<Vec<ForeignKeyEntry>, Error> {
        let rows = self
            .query_params(
                "SELECT k.constraint_name, k.column_name, k.referenced_table_schema, k.referenced_table_name, \
                        k.referenced_column_name, r.update_rule, r.delete_rule \
                 FROM information_schema.key_column_usage k \
                 JOIN information_schema.referential_constraints r \
                   ON r.constraint_schema = k.constraint_schema AND r.constraint_name = k.constraint_name \
                  AND r.table_name = k.table_name \
                 WHERE k.table_schema = ? AND k.table_name = ? AND k.referenced_table_name IS NOT NULL \
                 ORDER BY k.constraint_name, k.ordinal_position",
                (schema, table),
            )
            .await?;
        let mut keys: Vec<ForeignKeyEntry> = Vec::new();
        for mut row in rows {
            let name = cell(&mut row, 0);
            if keys.last().is_none_or(|k| k.name != name) {
                keys.push(ForeignKeyEntry {
                    name,
                    columns: Vec::new(),
                    referenced_schema: cell(&mut row, 2),
                    referenced_table: cell(&mut row, 3),
                    referenced_columns: Vec::new(),
                    on_update: cell(&mut row, 5),
                    on_delete: cell(&mut row, 6),
                });
            }
            let key = keys.last_mut().expect("just pushed");
            key.columns.push(cell(&mut row, 1));
            key.referenced_columns.push(cell(&mut row, 4));
        }
        Ok(keys)
    }

    /// Reads a table into the designer's model. `None` if it does not exist.
    pub async fn table_spec(&mut self, schema: &str, table: &str) -> Result<Option<TableSpec>, Error> {
        let Some(details) = self.table_details(schema, table).await? else {
            return Ok(None);
        };
        let mariadb = self.info().version.flavor == Flavor::MariaDb;

        let columns = self
            .query_params(
                "SELECT column_name, column_type, is_nullable, column_default, extra, column_comment, collation_name \
                 FROM information_schema.columns WHERE table_schema = ? AND table_name = ? ORDER BY ordinal_position",
                (schema, table),
            )
            .await?
            .into_iter()
            .map(|mut row| {
                let name = cell(&mut row, 0);
                let data_type = cell(&mut row, 1);
                let raw_default = row.get_mut(3).and_then(Option::take);
                let extra = cell(&mut row, 4);
                let extra_lower = extra.to_ascii_lowercase();
                let collation = row.get_mut(6).and_then(Option::take);

                // MariaDB reports defaults as SQL text ('abc', 0, NULL).
                // MySQL reports the bare value and flags expressions in `extra`.
                let default = raw_default.and_then(|value| {
                    if mariadb {
                        (!value.eq_ignore_ascii_case("NULL")).then_some(value)
                    } else if value.to_ascii_uppercase().starts_with("CURRENT_TIMESTAMP") {
                        Some(value)
                    } else if extra_lower.contains("default_generated") {
                        Some(format!("({value})"))
                    } else if is_numeric_type(&data_type) {
                        Some(value)
                    } else {
                        Some(crate::export::sql_literal(Some(value.as_bytes()), crate::store::ColumnKind::Text))
                    }
                });
                let on_update = extra_lower
                    .find("on update ")
                    .map(|at| extra[at + "on update ".len()..].trim().to_string());

                ColumnSpec {
                    original_name: Some(name.clone()),
                    name,
                    data_type,
                    nullable: cell(&mut row, 2).eq_ignore_ascii_case("YES"),
                    default,
                    on_update,
                    auto_increment: extra_lower.contains("auto_increment"),
                    comment: cell(&mut row, 5),
                    collation: collation.filter(|c| Some(c) != details.collation.as_ref()),
                    locked: extra_lower.contains("generated") && !extra_lower.contains("default_generated")
                        || extra_lower.contains("persistent"),
                }
            })
            .collect();

        let mut primary_key = Vec::new();
        let mut indexes: Vec<IndexSpec> = Vec::new();
        let rows = self
            .query_params(
                "SELECT index_name, non_unique, column_name, sub_part, index_type FROM information_schema.statistics \
                 WHERE table_schema = ? AND table_name = ? ORDER BY index_name, seq_in_index",
                (schema, table),
            )
            .await?;
        for mut row in rows {
            let name = cell(&mut row, 0);
            let column = row.get_mut(2).and_then(Option::take);
            let prefix = row.get_mut(3).and_then(Option::take);
            let index_type = cell(&mut row, 4).to_ascii_uppercase();
            if name == "PRIMARY" {
                primary_key.extend(column);
                continue;
            }
            if indexes.last().is_none_or(|i| i.name != name) {
                indexes.push(IndexSpec {
                    original_name: Some(name.clone()),
                    name,
                    columns: Vec::new(),
                    unique: cell(&mut row, 1) == "0",
                    locked: !matches!(index_type.as_str(), "BTREE" | "HASH"),
                });
            }
            let index = indexes.last_mut().expect("just pushed");
            // Prefix lengths and expression parts are beyond the designer.
            index.locked |= prefix.is_some() || column.is_none();
            index.columns.extend(column);
        }

        Ok(Some(TableSpec {
            schema: schema.to_string(),
            name: table.to_string(),
            columns,
            primary_key,
            indexes,
            engine: details.engine,
            collation: details.collation,
            comment: details.comment,
        }))
    }
}
