//! Writing a result out as CSV, TSV, JSON or SQL INSERT statements.
//!
//! Export reads straight from the result store and writes row by row, so it
//! works on results of any size and exports what is currently visible: with a
//! sort or filter active, the sorted or filtered rows.

use std::io::{self, Write};

use serde::Deserialize;

use crate::split::quote_identifier;
use crate::store::{ColumnInfo, ColumnKind, ResultStore};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Csv,
    Tsv,
    Json,
    Sql,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Csv => "csv",
            ExportFormat::Tsv => "tsv",
            ExportFormat::Json => "json",
            ExportFormat::Sql => "sql",
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(2 + bytes.len() * 2);
    out.push_str("0x");
    for b in bytes {
        out.push_str(&format!("{b:02X}"));
    }
    out
}

/// A plain decimal number, safe to write unquoted in JSON and SQL.
fn is_plain_number(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, "0"));
    !whole.is_empty()
        && whole.bytes().all(|b| b.is_ascii_digit())
        && !fraction.is_empty()
        && fraction.bytes().all(|b| b.is_ascii_digit())
        // JSON forbids leading zeros.
        && (whole.len() == 1 || !whole.starts_with('0'))
}

/// A value as a SQL literal.
pub fn sql_literal(value: Option<&[u8]>, kind: ColumnKind) -> String {
    let Some(bytes) = value else {
        return "NULL".to_string();
    };
    if kind == ColumnKind::Binary {
        return if bytes.is_empty() { "''".to_string() } else { hex(bytes) };
    }
    let text = String::from_utf8_lossy(bytes);
    if kind == ColumnKind::Number && is_plain_number(&text) {
        return text.into_owned();
    }
    let mut out = String::with_capacity(text.len() + 2);
    out.push('\'');
    for ch in text.chars() {
        match ch {
            '\'' => out.push_str("''"),
            '\\' => out.push_str("\\\\"),
            '\0' => out.push_str("\\0"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{1a}' => out.push_str("\\Z"),
            other => out.push(other),
        }
    }
    out.push('\'');
    out
}

fn csv_field(out: &mut impl Write, value: Option<&[u8]>, kind: ColumnKind) -> io::Result<()> {
    let Some(bytes) = value else {
        return Ok(()); // NULL is an empty, unquoted field
    };
    let text = if kind == ColumnKind::Binary {
        hex(bytes).into()
    } else {
        String::from_utf8_lossy(bytes)
    };
    // An empty string is quoted so it can be told apart from NULL.
    if text.is_empty() || text.contains([',', '"', '\n', '\r']) {
        write!(out, "\"{}\"", text.replace('"', "\"\""))
    } else {
        out.write_all(text.as_bytes())
    }
}

fn tsv_field(out: &mut impl Write, value: Option<&[u8]>, kind: ColumnKind) -> io::Result<()> {
    let Some(bytes) = value else {
        return out.write_all(b"\\N");
    };
    let text = if kind == ColumnKind::Binary {
        hex(bytes).into()
    } else {
        String::from_utf8_lossy(bytes)
    };
    let escaped = text
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    out.write_all(escaped.as_bytes())
}

fn json_value(value: Option<&[u8]>, kind: ColumnKind) -> serde_json::Value {
    let Some(bytes) = value else {
        return serde_json::Value::Null;
    };
    if kind == ColumnKind::Binary {
        return serde_json::Value::String(hex(bytes));
    }
    let text = String::from_utf8_lossy(bytes);
    match kind {
        ColumnKind::Number if is_plain_number(&text) => {
            serde_json::from_str(&text).unwrap_or_else(|_| serde_json::Value::String(text.into_owned()))
        }
        // A JSON column's content is embedded as JSON, not as a string of JSON.
        ColumnKind::Json => serde_json::from_str(&text).unwrap_or_else(|_| serde_json::Value::String(text.into_owned())),
        _ => serde_json::Value::String(text.into_owned()),
    }
}

/// Writes the store's visible rows to `out`. `table` names the target of the
/// INSERT statements and is ignored by the other formats. Returns the number
/// of rows written.
pub fn write(store: &ResultStore, format: ExportFormat, table: &str, out: &mut impl Write) -> io::Result<u64> {
    let columns: Vec<ColumnInfo> = store.status().columns;
    let kind = |index: usize| columns.get(index).map_or(ColumnKind::Text, |c| c.kind);

    match format {
        ExportFormat::Csv | ExportFormat::Tsv => {
            let separator: &[u8] = if format == ExportFormat::Csv { b"," } else { b"\t" };
            for (i, column) in columns.iter().enumerate() {
                if i > 0 {
                    out.write_all(separator)?;
                }
                if format == ExportFormat::Csv {
                    csv_field(out, Some(column.name.as_bytes()), ColumnKind::Text)?;
                } else {
                    tsv_field(out, Some(column.name.as_bytes()), ColumnKind::Text)?;
                }
            }
            out.write_all(b"\n")?;
            store.for_each_row(|cells| {
                for (i, cell) in cells.enumerate() {
                    if i > 0 {
                        out.write_all(separator)?;
                    }
                    if format == ExportFormat::Csv {
                        csv_field(out, cell, kind(i))?;
                    } else {
                        tsv_field(out, cell, kind(i))?;
                    }
                }
                out.write_all(b"\n")
            })
        }
        ExportFormat::Json => {
            out.write_all(b"[")?;
            let mut first = true;
            let rows = store.for_each_row(|cells| {
                let object: serde_json::Map<String, serde_json::Value> = cells
                    .enumerate()
                    .map(|(i, cell)| {
                        let name = columns.get(i).map_or_else(|| format!("column_{i}"), |c| c.name.clone());
                        (name, json_value(cell, kind(i)))
                    })
                    .collect();
                out.write_all(if first { b"\n  " } else { b",\n  " })?;
                first = false;
                serde_json::to_writer(&mut *out, &object).map_err(io::Error::other)
            })?;
            out.write_all(if rows == 0 { b"]\n" } else { b"\n]\n" })?;
            Ok(rows)
        }
        ExportFormat::Sql => {
            let target = quote_identifier(if table.trim().is_empty() { "table_name" } else { table.trim() });
            let names: Vec<String> = columns.iter().map(|c| quote_identifier(&c.name)).collect();
            let prefix = format!("INSERT INTO {target} ({}) VALUES (", names.join(", "));
            store.for_each_row(|cells| {
                out.write_all(prefix.as_bytes())?;
                for (i, cell) in cells.enumerate() {
                    if i > 0 {
                        out.write_all(b", ")?;
                    }
                    out.write_all(sql_literal(cell, kind(i)).as_bytes())?;
                }
                out.write_all(b");\n")
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::RowBatch;

    fn sample() -> ResultStore {
        let store = ResultStore::new();
        store.set_columns(vec![
            ColumnInfo::plain("id", ColumnKind::Number),
            ColumnInfo::plain("name", ColumnKind::Text),
            ColumnInfo::plain("note", ColumnKind::Text),
            ColumnInfo::plain("blob", ColumnKind::Binary),
            ColumnInfo::plain("doc", ColumnKind::Json),
        ]);
        let mut batch = RowBatch::default();
        batch.push_row([
            Some(&b"1"[..]),
            Some(&b"Ann, \"the\" first"[..]),
            Some(&b"line 1\nline 2\ttabbed"[..]),
            Some(&b"\x00\xFF"[..]),
            Some(&br#"{"a": [1, 2]}"#[..]),
        ]);
        batch.push_row([Some(&b"2.50"[..]), Some(&b"O'Brien \\ co"[..]), Some(&b""[..]), None, None]);
        batch.push_row([None, Some("Zoë".as_bytes()), None, Some(&b""[..]), Some(&b"not json"[..])]);
        store.append(&mut batch);
        store
    }

    fn export(format: ExportFormat) -> String {
        let mut out = Vec::new();
        let rows = write(&sample(), format, "people", &mut out).unwrap();
        assert_eq!(rows, 3);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn csv_quotes_only_where_needed_and_tells_null_from_empty() {
        assert_eq!(
            export(ExportFormat::Csv),
            "id,name,note,blob,doc\n\
             1,\"Ann, \"\"the\"\" first\",\"line 1\nline 2\ttabbed\",0x00FF,\"{\"\"a\"\": [1, 2]}\"\n\
             2.50,O'Brien \\ co,\"\",,\n\
             ,Zoë,,0x,not json\n"
        );
    }

    #[test]
    fn tsv_escapes_tabs_and_newlines_and_marks_null() {
        assert_eq!(
            export(ExportFormat::Tsv),
            "id\tname\tnote\tblob\tdoc\n\
             1\tAnn, \"the\" first\tline 1\\nline 2\\ttabbed\t0x00FF\t{\"a\": [1, 2]}\n\
             2.50\tO'Brien \\\\ co\t\t\\N\t\\N\n\
             \\N\tZoë\t\\N\t0x\tnot json\n"
        );
    }

    #[test]
    fn json_keeps_numbers_and_embeds_json_columns() {
        let parsed: serde_json::Value = serde_json::from_str(&export(ExportFormat::Json)).unwrap();
        assert_eq!(
            parsed,
            serde_json::json!([
                {"id": 1, "name": "Ann, \"the\" first", "note": "line 1\nline 2\ttabbed", "blob": "0x00FF", "doc": {"a": [1, 2]}},
                {"id": 2.50, "name": "O'Brien \\ co", "note": "", "blob": null, "doc": null},
                {"id": null, "name": "Zoë", "note": null, "blob": "0x", "doc": "not json"},
            ])
        );
    }

    #[test]
    fn sql_inserts_quote_and_escape_values() {
        assert_eq!(
            export(ExportFormat::Sql),
            "INSERT INTO `people` (`id`, `name`, `note`, `blob`, `doc`) VALUES (1, 'Ann, \"the\" first', 'line 1\\nline 2\ttabbed', 0x00FF, '{\"a\": [1, 2]}');\n\
             INSERT INTO `people` (`id`, `name`, `note`, `blob`, `doc`) VALUES (2.50, 'O''Brien \\\\ co', '', NULL, NULL);\n\
             INSERT INTO `people` (`id`, `name`, `note`, `blob`, `doc`) VALUES (NULL, 'Zoë', NULL, '', 'not json');\n"
        );
    }

    #[test]
    fn an_empty_result_still_has_a_header_or_brackets() {
        let store = ResultStore::new();
        store.set_columns(vec![ColumnInfo::plain("id", ColumnKind::Number)]);
        let mut out = Vec::new();
        assert_eq!(write(&store, ExportFormat::Csv, "", &mut out).unwrap(), 0);
        assert_eq!(out, b"id\n");
        let mut out = Vec::new();
        write(&store, ExportFormat::Json, "", &mut out).unwrap();
        assert_eq!(out, b"[]\n");
    }

    #[test]
    fn export_follows_the_current_sort_and_filter() {
        let store = sample();
        store.set_view(Some("o"), Some((0, true)));
        let mut out = Vec::new();
        assert_eq!(write(&store, ExportFormat::Tsv, "", &mut out).unwrap(), 2);
        let text = String::from_utf8(out).unwrap();
        let ids: Vec<&str> = text.lines().skip(1).map(|l| l.split('\t').next().unwrap()).collect();
        assert_eq!(ids, ["2.50", "\\N"], "rows containing an o, highest id first");
    }

    #[test]
    fn only_plain_decimals_are_written_unquoted() {
        for (text, plain) in [("12", true), ("-3.50", true), ("0", true), ("007", false), ("1e5", false), ("NaN", false), ("", false), ("1.", false)] {
            assert_eq!(is_plain_number(text), plain, "{text:?}");
        }
        assert_eq!(sql_literal(Some(b"007"), ColumnKind::Number), "'007'");
    }
}
