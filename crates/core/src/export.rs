//! Writing a result out as CSV, TSV, JSON or SQL INSERT statements.
//!
//! Export reads straight from the result store and writes row by row, so it
//! works on results of any size and exports what is currently visible: with a
//! sort or filter active, the sorted or filtered rows.

use std::io::{self, Write};

use serde::{Deserialize, Serialize};

use crate::error::Error;
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
    let text = if kind == ColumnKind::Binary { hex(bytes).into() } else { String::from_utf8_lossy(bytes) };
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
    let text = if kind == ColumnKind::Binary { hex(bytes).into() } else { String::from_utf8_lossy(bytes) };
    let escaped = text.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n").replace('\r', "\\r");
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

/// Writes rows in one export format, one row at a time. The rows can come
/// from a result store or straight off the connection, so a result far
/// larger than memory can be exported.
pub struct RowWriter {
    format: ExportFormat,
    columns: Vec<ColumnInfo>,
    /// The start of each INSERT statement.
    insert: String,
    rows: u64,
}

impl RowWriter {
    /// `table` names the target of the INSERT statements and is ignored by the other formats.
    pub fn new(format: ExportFormat, columns: Vec<ColumnInfo>, table: &str) -> Self {
        let target = quote_identifier(if table.trim().is_empty() { "table_name" } else { table.trim() });
        let names: Vec<String> = columns.iter().map(|c| quote_identifier(&c.name)).collect();
        Self { format, insert: format!("INSERT INTO {target} ({}) VALUES (", names.join(", ")), columns, rows: 0 }
    }

    fn kind(&self, index: usize) -> ColumnKind {
        self.columns.get(index).map_or(ColumnKind::Text, |c| c.kind)
    }

    /// Writes whatever comes before the first row: the header line, or the opening bracket.
    pub fn begin(&mut self, out: &mut impl Write) -> io::Result<()> {
        match self.format {
            ExportFormat::Csv | ExportFormat::Tsv => {
                for (i, column) in self.columns.iter().enumerate() {
                    if i > 0 {
                        out.write_all(if self.format == ExportFormat::Csv { b"," } else { b"\t" })?;
                    }
                    if self.format == ExportFormat::Csv {
                        csv_field(out, Some(column.name.as_bytes()), ColumnKind::Text)?;
                    } else {
                        tsv_field(out, Some(column.name.as_bytes()), ColumnKind::Text)?;
                    }
                }
                out.write_all(b"\n")
            }
            ExportFormat::Json => out.write_all(b"["),
            ExportFormat::Sql => Ok(()),
        }
    }

    pub fn row<'a>(&mut self, out: &mut impl Write, cells: impl Iterator<Item = Option<&'a [u8]>>) -> io::Result<()> {
        match self.format {
            ExportFormat::Csv | ExportFormat::Tsv => {
                for (i, cell) in cells.enumerate() {
                    if i > 0 {
                        out.write_all(if self.format == ExportFormat::Csv { b"," } else { b"\t" })?;
                    }
                    if self.format == ExportFormat::Csv {
                        csv_field(out, cell, self.kind(i))?;
                    } else {
                        tsv_field(out, cell, self.kind(i))?;
                    }
                }
                out.write_all(b"\n")?;
            }
            ExportFormat::Json => {
                let object: serde_json::Map<String, serde_json::Value> = cells
                    .enumerate()
                    .map(|(i, cell)| {
                        let name = self.columns.get(i).map_or_else(|| format!("column_{i}"), |c| c.name.clone());
                        (name, json_value(cell, self.kind(i)))
                    })
                    .collect();
                out.write_all(if self.rows == 0 { b"\n  " } else { b",\n  " })?;
                serde_json::to_writer(&mut *out, &object).map_err(io::Error::other)?;
            }
            ExportFormat::Sql => {
                out.write_all(self.insert.as_bytes())?;
                for (i, cell) in cells.enumerate() {
                    if i > 0 {
                        out.write_all(b", ")?;
                    }
                    out.write_all(sql_literal(cell, self.kind(i)).as_bytes())?;
                }
                out.write_all(b");\n")?;
            }
        }
        self.rows += 1;
        Ok(())
    }

    /// Writes whatever comes after the last row. Returns the number of rows written.
    pub fn end(&mut self, out: &mut impl Write) -> io::Result<u64> {
        if self.format == ExportFormat::Json {
            out.write_all(if self.rows == 0 { b"]\n" } else { b"\n]\n" })?;
        }
        Ok(self.rows)
    }
}

/// The name to offer for an exported file, without its extension, made from
/// what the interface suggests: the table the rows came from, or the script.
/// Whatever a file system would refuse, or read as a path, is taken out; if
/// nothing usable is left the name is `result`. The user still chooses the
/// folder and may change the name in the Save dialog.
pub fn file_stem(suggested: Option<&str>) -> String {
    const FALLBACK: &str = "result";
    // Names Windows keeps for devices, with or without an extension.
    const RESERVED: [&str; 22] = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4",
        "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    let spaced: String =
        suggested.unwrap_or_default().chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { ' ' } else { c }).collect();
    let joined = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = joined.trim_matches(['.', ' ']).chars().take(80).collect();
    let stem = cut.trim_end_matches(['.', ' ']);
    let device = stem.split('.').next().unwrap_or_default().trim().to_ascii_lowercase();
    if stem.is_empty() || RESERVED.contains(&device.as_str()) {
        FALLBACK.to_string()
    } else {
        stem.to_string()
    }
}

/// Writes the store's visible rows to `out`. `table` names the target of the
/// INSERT statements and is ignored by the other formats. Returns the number
/// of rows written.
///
/// This is what the store holds, which is not always everything the query
/// returns: a result stopped at the row limit holds only the rows up to it.
/// To export the whole result of a query, see [`crate::Session::export`].
pub fn write(store: &ResultStore, format: ExportFormat, table: &str, out: &mut impl Write) -> io::Result<u64> {
    let mut writer = RowWriter::new(format, store.status().columns, table);
    writer.begin(out)?;
    store.for_each_row(|cells| writer.row(&mut *out, cells))?;
    writer.end(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exported_file_is_named_after_what_it_holds_and_never_as_a_path() {
        assert_eq!(file_stem(None), "result");
        assert_eq!(file_stem(Some("   ")), "result");
        assert_eq!(file_stem(Some("people")), "people");
        assert_eq!(file_stem(Some("Zo\u{eb}'s t\u{e4}ble")), "Zo\u{eb}'s t\u{e4}ble");
        assert_eq!(file_stem(Some("sales/2026: q4?")), "sales 2026 q4");
        // Nothing that could step out of the folder the user picks.
        assert_eq!(file_stem(Some("..\\..\\etc\\passwd")), "etc passwd");
        assert_eq!(file_stem(Some("../../x")), "x");
        assert_eq!(file_stem(Some("line\nbreak\ttab")), "line break tab");
        // Device names on Windows, whatever follows the first dot.
        assert_eq!(file_stem(Some("NUL")), "result");
        assert_eq!(file_stem(Some("con.2026")), "result");
        assert_eq!(file_stem(Some("console")), "console");
        // Not too long for a file name, and no dot or space left at the end.
        assert_eq!(file_stem(Some(&"x".repeat(300))).chars().count(), 80);
        assert_eq!(file_stem(Some(&format!("{}. tail", "y".repeat(78)))), "y".repeat(78));
        assert_eq!(file_stem(Some("...hidden...")), "hidden");
    }
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
        for (text, plain) in
            [("12", true), ("-3.50", true), ("0", true), ("007", false), ("1e5", false), ("NaN", false), ("", false), ("1.", false)]
        {
            assert_eq!(is_plain_number(text), plain, "{text:?}");
        }
        assert_eq!(sql_literal(Some(b"007"), ColumnKind::Number), "'007'");
    }
}

/// A rectangle of cells in the visible rows. Both ends are included.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellRange {
    pub row_start: u64,
    pub row_end: u64,
    pub col_start: usize,
    pub col_end: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CopyFormat {
    /// Tab-separated, the form spreadsheets paste as cells.
    Tsv,
    Csv,
    /// One INSERT statement per row.
    Sql,
    /// Quoted values separated by commas, for an `IN (...)` list.
    List,
}

/// More than this is refused; a file export has no such limit.
const MAX_COPY_BYTES: usize = 32 * 1024 * 1024;

/// A field as a spreadsheet expects it on the clipboard: quoted only if it
/// contains a tab, a line break or a quote.
fn clipboard_field(value: Option<&[u8]>, kind: ColumnKind) -> String {
    let Some(bytes) = value else {
        return "NULL".to_string();
    };
    let text = if kind == ColumnKind::Binary { hex(bytes).into() } else { String::from_utf8_lossy(bytes) };
    if text.contains(['\t', '\n', '\r', '"']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.into_owned()
    }
}

/// The cells of `range` as text for the clipboard.
pub fn copy_range(store: &ResultStore, range: CellRange, format: CopyFormat, headers: bool, table: &str) -> Result<String, Error> {
    let columns: Vec<ColumnInfo> = store.status().columns;
    let last_column = range.col_end.min(columns.len().saturating_sub(1));
    if columns.is_empty() || range.col_start > last_column || range.row_start > range.row_end {
        return Ok(String::new());
    }
    let selected = &columns[range.col_start..=last_column];

    // One cell is copied as exactly its text. The quoting that keeps cells
    // apart would only get in the way when pasting a value into a query.
    if format == CopyFormat::Tsv && !headers && range.row_start == range.row_end && selected.len() == 1 {
        return Ok(match store.cell(range.row_start, range.col_start) {
            Some(Some(bytes)) if selected[0].kind == ColumnKind::Binary => hex(&bytes),
            Some(Some(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
            Some(None) => "NULL".to_string(),
            None => String::new(),
        });
    }
    let mut out: Vec<u8> = Vec::new();

    if headers && matches!(format, CopyFormat::Tsv | CopyFormat::Csv) {
        for (i, column) in selected.iter().enumerate() {
            if i > 0 {
                out.push(if format == CopyFormat::Csv { b',' } else { b'\t' });
            }
            if format == CopyFormat::Csv {
                csv_field(&mut out, Some(column.name.as_bytes()), ColumnKind::Text).expect("writing to memory");
            } else {
                out.extend_from_slice(clipboard_field(Some(column.name.as_bytes()), ColumnKind::Text).as_bytes());
            }
        }
        out.push(b'\n');
    }

    let target = quote_identifier(if table.trim().is_empty() { "table_name" } else { table.trim() });
    let names: Vec<String> = selected.iter().map(|c| quote_identifier(&c.name)).collect();
    let mut first_value = true;
    store.for_each_row_in(range.row_start, range.row_end, |cells| {
        if out.len() > MAX_COPY_BYTES {
            return Err(Error::Other("That selection is too large to copy. Export it to a file instead.".into()));
        }
        let picked = cells.enumerate().skip(range.col_start).take(selected.len());
        match format {
            CopyFormat::Tsv | CopyFormat::Csv => {
                for (n, (i, cell)) in picked.enumerate() {
                    if n > 0 {
                        out.push(if format == CopyFormat::Csv { b',' } else { b'\t' });
                    }
                    if format == CopyFormat::Csv {
                        csv_field(&mut out, cell, columns[i].kind).expect("writing to memory");
                    } else {
                        out.extend_from_slice(clipboard_field(cell, columns[i].kind).as_bytes());
                    }
                }
                out.push(b'\n');
            }
            CopyFormat::Sql => {
                out.extend_from_slice(format!("INSERT INTO {target} ({}) VALUES (", names.join(", ")).as_bytes());
                for (n, (i, cell)) in picked.enumerate() {
                    if n > 0 {
                        out.extend_from_slice(b", ");
                    }
                    out.extend_from_slice(sql_literal(cell, columns[i].kind).as_bytes());
                }
                out.extend_from_slice(b");\n");
            }
            CopyFormat::List => {
                for (i, cell) in picked {
                    if !first_value {
                        out.extend_from_slice(b", ");
                    }
                    first_value = false;
                    out.extend_from_slice(sql_literal(cell, columns[i].kind).as_bytes());
                }
            }
        }
        Ok(())
    })?;

    // A single line needs no trailing line break; it would paste as an extra row.
    if out.last() == Some(&b'\n') {
        out.pop();
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Totals for a selection, shown under the grid.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RangeStats {
    pub rows: u64,
    pub cells: u64,
    /// How many of the cells hold a number.
    pub numbers: u64,
    pub sum: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub average: Option<f64>,
}

/// Counts and adds up the cells of `range`.
pub fn range_stats(store: &ResultStore, range: CellRange) -> RangeStats {
    let mut stats = RangeStats::default();
    let width = range.col_end.saturating_sub(range.col_start) + 1;
    let mut sum = 0f64;
    let _ = store.for_each_row_in(range.row_start, range.row_end, |cells| {
        stats.rows += 1;
        for cell in cells.skip(range.col_start).take(width) {
            stats.cells += 1;
            let number = cell
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .and_then(|text| text.trim().parse::<f64>().ok())
                .filter(|n| n.is_finite());
            if let Some(n) = number {
                stats.numbers += 1;
                sum += n;
                stats.min = Some(stats.min.map_or(n, |m| m.min(n)));
                stats.max = Some(stats.max.map_or(n, |m| m.max(n)));
            }
        }
        Ok::<(), ()>(())
    });
    if stats.numbers > 0 {
        stats.sum = Some(sum);
        stats.average = Some(sum / stats.numbers as f64);
    }
    stats
}

#[cfg(test)]
mod range_tests {
    use super::*;
    use crate::store::RowBatch;

    fn sample() -> ResultStore {
        let store = ResultStore::new();
        store.set_columns(vec![
            ColumnInfo::plain("id", ColumnKind::Number),
            ColumnInfo::plain("name", ColumnKind::Text),
            ColumnInfo::plain("amount", ColumnKind::Number),
        ]);
        let mut batch = RowBatch::default();
        batch.push_row([Some(&b"1"[..]), Some(&b"Ann"[..]), Some(&b"10.5"[..])]);
        batch.push_row([Some(&b"2"[..]), Some(&b"O'Brien\tJr"[..]), None]);
        batch.push_row([Some(&b"3"[..]), Some(&b"line\nbreak \"q\""[..]), Some(&b"4"[..])]);
        store.append(&mut batch);
        store
    }

    fn range(row_start: u64, row_end: u64, col_start: usize, col_end: usize) -> CellRange {
        CellRange { row_start, row_end, col_start, col_end }
    }

    #[test]
    fn tsv_pastes_into_a_spreadsheet_as_cells() {
        let text = copy_range(&sample(), range(0, 2, 0, 2), CopyFormat::Tsv, true, "").unwrap();
        assert_eq!(text, "id\tname\tamount\n1\tAnn\t10.5\n2\t\"O'Brien\tJr\"\tNULL\n3\t\"line\nbreak \"\"q\"\"\"\t4");
    }

    #[test]
    fn one_cell_copies_as_just_its_text() {
        assert_eq!(copy_range(&sample(), range(0, 0, 1, 1), CopyFormat::Tsv, false, "").unwrap(), "Ann");
        // Not quoted, even with a tab or line break in it.
        assert_eq!(copy_range(&sample(), range(1, 1, 1, 1), CopyFormat::Tsv, false, "").unwrap(), "O'Brien\tJr");
        assert_eq!(copy_range(&sample(), range(1, 1, 2, 2), CopyFormat::Tsv, false, "").unwrap(), "NULL");
        // Follows the sort like everything else.
        let sorted = sample();
        sorted.set_view(None, Some((0, true)));
        assert_eq!(copy_range(&sorted, range(0, 0, 1, 1), CopyFormat::Tsv, false, "").unwrap(), "line\nbreak \"q\"");
    }

    #[test]
    fn a_sub_rectangle_keeps_only_its_columns() {
        let text = copy_range(&sample(), range(1, 2, 0, 0), CopyFormat::Csv, true, "").unwrap();
        assert_eq!(text, "id\n2\n3");
    }

    #[test]
    fn list_format_is_ready_for_an_in_clause() {
        assert_eq!(copy_range(&sample(), range(0, 2, 0, 0), CopyFormat::List, false, "").unwrap(), "1, 2, 3");
        assert_eq!(copy_range(&sample(), range(0, 1, 1, 1), CopyFormat::List, false, "").unwrap(), "'Ann', 'O''Brien\tJr'");
    }

    #[test]
    fn sql_format_writes_one_insert_per_row_for_the_selected_columns() {
        let text = copy_range(&sample(), range(0, 1, 0, 1), CopyFormat::Sql, false, "people").unwrap();
        assert_eq!(
            text,
            "INSERT INTO `people` (`id`, `name`) VALUES (1, 'Ann');\nINSERT INTO `people` (`id`, `name`) VALUES (2, 'O''Brien\tJr');"
        );
    }

    #[test]
    fn ranges_past_the_edges_are_clamped() {
        assert_eq!(copy_range(&sample(), range(2, 99, 0, 99), CopyFormat::Tsv, false, "").unwrap().lines().count(), 2);
        assert_eq!(copy_range(&sample(), range(50, 60, 0, 0), CopyFormat::Tsv, false, "").unwrap(), "");
        assert_eq!(copy_range(&sample(), range(0, 0, 9, 9), CopyFormat::Tsv, false, "").unwrap(), "");
    }

    #[test]
    fn copying_follows_the_current_sort() {
        let store = sample();
        store.set_view(None, Some((0, true)));
        assert_eq!(copy_range(&store, range(0, 0, 0, 0), CopyFormat::Tsv, false, "").unwrap(), "3");
    }

    #[test]
    fn stats_add_up_numbers_and_ignore_text_and_null() {
        let stats = range_stats(&sample(), range(0, 2, 0, 2));
        assert_eq!((stats.rows, stats.cells, stats.numbers), (3, 9, 5));
        assert_eq!(stats.sum, Some(20.5));
        assert_eq!((stats.min, stats.max), (Some(1.0), Some(10.5)));
        assert_eq!(stats.average, Some(4.1));

        let text_only = range_stats(&sample(), range(0, 2, 1, 1));
        assert_eq!((text_only.cells, text_only.numbers, text_only.sum), (3, 0, None));
    }
}
