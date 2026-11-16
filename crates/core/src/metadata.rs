//! Reading what exists on the server: schemas, tables, columns, routines.
//!
//! Every query is limited to one schema. Unfiltered `information_schema`
//! queries are slow on large MySQL 5.7 servers, so nothing here scans it
//! whole. Names are passed as parameters, never spliced into the SQL.

use serde::Serialize;

use crate::error::Error;
use crate::session::Session;

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
