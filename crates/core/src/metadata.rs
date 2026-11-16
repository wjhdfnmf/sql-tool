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
