//! In-memory storage for a query result.
//!
//! The result lives here, not in the user interface. The UI asks for windows
//! of rows as the user scrolls. Rows are kept in the same compact form the
//! server sent them: one length-prefixed byte string per cell, back to back in
//! a single buffer, so a window of rows is one contiguous slice.
//!
//! Sorting and filtering do not move any data. They build a *view*: a list
//! of row numbers in display order. Everything the UI reads goes through it.
//!
//! Cell encoding (the MySQL length-encoded string):
//! - `0xFB`                      NULL
//! - `n` where n < 251, n bytes  short value
//! - `0xFC`, u16 length, bytes
//! - `0xFD`, u24 length, bytes
//! - `0xFE`, u64 length, bytes

use std::cmp::Ordering;
use std::sync::RwLock;

use serde::Serialize;

/// Broad category of a column, enough for the UI to choose alignment and
/// how to show a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ColumnKind {
    Number,
    Text,
    Date,
    Json,
    Binary,
}

/// The table column a result column was read from, when the server says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnSource {
    pub schema: String,
    pub table: String,
    pub column: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub name: String,
    pub kind: ColumnKind,
    /// `None` for expressions and other computed columns.
    pub source: Option<ColumnSource>,
    /// Part of the source table's primary key.
    pub primary_key: bool,
}

impl ColumnInfo {
    /// A column that is not tied to a table, as tests and computed results use.
    pub fn plain(name: &str, kind: ColumnKind) -> Self {
        Self {
            name: name.to_string(),
            kind,
            source: None,
            primary_key: false,
        }
    }
}

/// Rows collected by the reader before being handed to the store in one go,
/// so the store's lock is taken once per batch instead of once per row.
#[derive(Default)]
pub struct RowBatch {
    data: Vec<u8>,
    /// End offset of each row within `data`.
    ends: Vec<u32>,
}

impl RowBatch {
    pub fn rows(&self) -> usize {
        self.ends.len()
    }

    /// Appends one row. `None` is NULL.
    pub fn push_row<'a>(&mut self, cells: impl IntoIterator<Item = Option<&'a [u8]>>) {
        for cell in cells {
            self.push_cell(cell);
        }
        self.end_row();
    }

    /// Appends one cell to the row being built. Finish it with [`RowBatch::end_row`].
    pub fn push_cell(&mut self, cell: Option<&[u8]>) {
        match cell {
            None => self.data.push(0xFB),
            Some(bytes) => {
                put_len(&mut self.data, bytes.len());
                self.data.extend_from_slice(bytes);
            }
        }
    }

    pub fn end_row(&mut self) {
        self.ends.push(self.data.len() as u32);
    }

    fn clear(&mut self) {
        self.data.clear();
        self.ends.clear();
    }
}

fn put_len(buf: &mut Vec<u8>, n: usize) {
    match n {
        0..=250 => buf.push(n as u8),
        251..=0xFFFF => {
            buf.push(0xFC);
            buf.extend_from_slice(&(n as u16).to_le_bytes());
        }
        0x1_0000..=0xFF_FFFF => {
            buf.push(0xFD);
            buf.extend_from_slice(&(n as u32).to_le_bytes()[..3]);
        }
        _ => {
            buf.push(0xFE);
            buf.extend_from_slice(&(n as u64).to_le_bytes());
        }
    }
}

/// Reads one cell from the front of `row`, returning it and the rest.
fn take_cell(row: &[u8]) -> Option<(Option<&[u8]>, &[u8])> {
    let (&first, tail) = row.split_first()?;
    let (len, header) = match first {
        0xFB => return Some((None, tail)),
        0xFC => (u16::from_le_bytes(tail.get(..2)?.try_into().ok()?) as usize, 2),
        0xFD => {
            let b = tail.get(..3)?;
            (u32::from_le_bytes([b[0], b[1], b[2], 0]) as usize, 3)
        }
        0xFE => (u64::from_le_bytes(tail.get(..8)?.try_into().ok()?) as usize, 8),
        n => (n as usize, 0),
    };
    Some((Some(tail.get(header..header + len)?), &tail[header + len..]))
}

/// The cells of one encoded row.
pub(crate) struct Cells<'a>(&'a [u8]);

impl<'a> Iterator for Cells<'a> {
    type Item = Option<&'a [u8]>;

    fn next(&mut self) -> Option<Self::Item> {
        let (cell, rest) = take_cell(self.0)?;
        self.0 = rest;
        Some(cell)
    }
}

fn contains_ignore_ascii_case(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty() || haystack.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoreStatus {
    pub columns: Vec<ColumnInfo>,
    /// Rows currently visible: all of them, or those that pass the filter.
    pub rows: u64,
    /// Rows in the result regardless of any filter.
    pub total_rows: u64,
    pub bytes: u64,
    /// True once the query has finished, successfully or not.
    pub done: bool,
    /// True when the row limit stopped the result before the server was done.
    pub truncated: bool,
    pub error: Option<String>,
}

#[derive(Default)]
struct Inner {
    columns: Vec<ColumnInfo>,
    truncated: bool,
    data: Vec<u8>,
    /// Start offset of each row, plus one final entry for the end of the last
    /// row, so row `i` is `data[offsets[i]..offsets[i + 1]]`.
    offsets: Vec<u64>,
    /// Row numbers in display order while a sort or filter is active.
    view: Option<Vec<u32>>,
    done: bool,
    error: Option<String>,
}

impl Inner {
    fn total(&self) -> u64 {
        self.offsets.len() as u64 - 1
    }

    fn visible(&self) -> u64 {
        self.view.as_ref().map_or(self.total(), |v| v.len() as u64)
    }

    fn raw_row(&self, row: usize) -> &[u8] {
        &self.data[self.offsets[row] as usize..self.offsets[row + 1] as usize]
    }

    /// The stored row behind a visible row number.
    fn resolve(&self, visible_row: u64) -> Option<usize> {
        match &self.view {
            Some(view) => view.get(visible_row as usize).map(|r| *r as usize),
            None => (visible_row < self.total()).then_some(visible_row as usize),
        }
    }
}

/// One result set. Written by the query reader, read by the UI, concurrently.
pub struct ResultStore {
    inner: RwLock<Inner>,
}

impl Default for ResultStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ResultStore {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Inner {
                offsets: vec![0],
                ..Inner::default()
            }),
        }
    }

    pub fn set_columns(&self, columns: Vec<ColumnInfo>) {
        self.inner.write().unwrap().columns = columns;
    }

    pub fn set_truncated(&self) {
        self.inner.write().unwrap().truncated = true;
    }

    /// Moves the batch's rows into the store and empties the batch.
    pub fn append(&self, batch: &mut RowBatch) {
        if batch.ends.is_empty() {
            return;
        }
        let mut inner = self.inner.write().unwrap();
        let base = inner.data.len() as u64;
        inner.data.extend_from_slice(&batch.data);
        inner.offsets.extend(batch.ends.iter().map(|end| base + *end as u64));
        drop(inner);
        batch.clear();
    }

    pub fn finish(&self, error: Option<String>) {
        let mut inner = self.inner.write().unwrap();
        inner.done = true;
        inner.error = error;
    }

    pub fn status(&self) -> StoreStatus {
        let inner = self.inner.read().unwrap();
        StoreStatus {
            columns: inner.columns.clone(),
            rows: inner.visible(),
            total_rows: inner.total(),
            bytes: inner.data.len() as u64,
            done: inner.done,
            truncated: inner.truncated,
            error: inner.error.clone(),
        }
    }

    /// Returns up to `count` visible rows starting at `offset`, ready to send
    /// to the UI: a little-endian `u32` row count, then the encoded rows.
    pub fn window(&self, offset: u64, count: u32) -> Vec<u8> {
        let inner = self.inner.read().unwrap();
        let visible = inner.visible();
        let first = offset.min(visible);
        let last = first.saturating_add(count as u64).min(visible);
        let mut out = Vec::new();
        out.extend_from_slice(&((last - first) as u32).to_le_bytes());
        match &inner.view {
            None => out.extend_from_slice(
                &inner.data[inner.offsets[first as usize] as usize..inner.offsets[last as usize] as usize],
            ),
            Some(view) => {
                for row in &view[first as usize..last as usize] {
                    out.extend_from_slice(inner.raw_row(*row as usize));
                }
            }
        }
        out
    }

    /// The full value of one cell of a visible row: `None` if the row or
    /// column does not exist, `Some(None)` for SQL NULL.
    pub fn cell(&self, row: u64, column: usize) -> Option<Option<Vec<u8>>> {
        let inner = self.inner.read().unwrap();
        let raw = inner.raw_row(inner.resolve(row)?);
        Cells(raw).nth(column).map(|cell| cell.map(<[u8]>::to_vec))
    }

    /// Every cell of one visible row.
    pub fn row(&self, row: u64) -> Option<Vec<Option<Vec<u8>>>> {
        let inner = self.inner.read().unwrap();
        let raw = inner.raw_row(inner.resolve(row)?);
        Some(Cells(raw).map(|cell| cell.map(<[u8]>::to_vec)).collect())
    }

    /// Calls `visit` with each visible row's cells, in display order.
    pub(crate) fn for_each_row<E>(&self, mut visit: impl FnMut(Cells<'_>) -> Result<(), E>) -> Result<u64, E> {
        let inner = self.inner.read().unwrap();
        let visible = inner.visible();
        for index in 0..visible {
            let row = inner.resolve(index).expect("index is within the view");
            visit(Cells(inner.raw_row(row)))?;
        }
        Ok(visible)
    }

    /// Sorts and filters what the UI sees, without changing the stored rows.
    ///
    /// `filter` keeps rows where any cell contains the text (ASCII letters
    /// match regardless of case). `sort` is a column and whether to sort
    /// descending; numeric columns sort by value, others as text, and NULLs
    /// sort before everything else. Returns how many rows are visible.
    pub fn set_view(&self, filter: Option<&str>, sort: Option<(usize, bool)>) -> u64 {
        let mut inner = self.inner.write().unwrap();
        let needle = filter.map(str::trim).filter(|f| !f.is_empty());
        if needle.is_none() && sort.is_none() {
            inner.view = None;
            return inner.total();
        }

        let total = inner.total() as usize;
        let mut rows: Vec<u32> = match needle {
            None => (0..total as u32).collect(),
            Some(needle) => (0..total)
                .filter(|row| {
                    Cells(inner.raw_row(*row))
                        .flatten()
                        .any(|cell| contains_ignore_ascii_case(cell, needle.as_bytes()))
                })
                .map(|row| row as u32)
                .collect(),
        };

        if let Some((column, descending)) = sort {
            let numeric = inner.columns.get(column).is_some_and(|c| c.kind == ColumnKind::Number);
            let cell = |row: u32| Cells(inner.raw_row(row as usize)).nth(column).flatten();
            let compare = |a: &u32, b: &u32| -> Ordering {
                let ordering = match (cell(*a), cell(*b)) {
                    (None, None) => Ordering::Equal,
                    (None, Some(_)) => Ordering::Less,
                    (Some(_), None) => Ordering::Greater,
                    (Some(x), Some(y)) if numeric => {
                        let parse = |v: &[u8]| std::str::from_utf8(v).ok().and_then(|s| s.trim().parse::<f64>().ok());
                        match (parse(x), parse(y)) {
                            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
                            _ => x.cmp(y),
                        }
                    }
                    (Some(x), Some(y)) => x
                        .iter()
                        .map(u8::to_ascii_lowercase)
                        .cmp(y.iter().map(u8::to_ascii_lowercase))
                        .then_with(|| x.cmp(y)),
                };
                if descending { ordering.reverse() } else { ordering }
            };
            // Stable, so rows that compare equal keep the order they arrived in.
            rows.sort_by(compare);
        }

        let visible = rows.len() as u64;
        inner.view = Some(rows);
        visible
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with(rows: &[&[Option<&str>]]) -> ResultStore {
        let store = ResultStore::new();
        let mut batch = RowBatch::default();
        for row in rows {
            batch.push_row(row.iter().map(|c| c.map(str::as_bytes)));
        }
        store.append(&mut batch);
        assert_eq!(batch.rows(), 0, "append empties the batch");
        store
    }

    fn column(store: &ResultStore, index: usize) -> Vec<Option<String>> {
        (0..store.status().rows)
            .map(|row| store.cell(row, index).unwrap().map(|b| String::from_utf8(b).unwrap()))
            .collect()
    }

    #[test]
    fn window_returns_requested_rows_in_wire_format() {
        let store = store_with(&[&[Some("1"), Some("ann")], &[Some("2"), None], &[Some("3"), Some("")]]);
        assert_eq!(
            store.window(1, 2),
            [
                &2u32.to_le_bytes()[..],
                &[1, b'2', 0xFB], // row 2: "2", NULL
                &[1, b'3', 0],    // row 3: "3", empty string
            ]
            .concat()
        );
    }

    #[test]
    fn window_is_clamped_to_available_rows() {
        let store = store_with(&[&[Some("a")], &[Some("b")]]);
        assert_eq!(store.window(1, 100), [&1u32.to_le_bytes()[..], &[1, b'b']].concat());
        assert_eq!(store.window(2, 10), 0u32.to_le_bytes());
        assert_eq!(store.window(9_999, 10), 0u32.to_le_bytes());
    }

    #[test]
    fn rows_from_separate_batches_are_contiguous() {
        let store = store_with(&[&[Some("a")]]);
        let mut batch = RowBatch::default();
        batch.push_row([Some(&b"b"[..])]);
        batch.push_row([Some(&b"c"[..])]);
        store.append(&mut batch);
        assert_eq!(store.status().rows, 3);
        assert_eq!(
            store.window(0, 3),
            [&3u32.to_le_bytes()[..], &[1, b'a', 1, b'b', 1, b'c']].concat()
        );
    }

    #[test]
    fn long_values_use_the_wider_length_prefixes() {
        let long = "x".repeat(300);
        let store = store_with(&[&[Some(long.as_str())]]);
        let w = store.window(0, 1);
        assert_eq!(&w[4..7], &[0xFC, 0x2C, 0x01], "0xFC then 300 as u16");
        assert_eq!(w.len(), 4 + 3 + 300);
    }

    #[test]
    fn cell_returns_full_values_nulls_and_missing() {
        let long = "y".repeat(70_000);
        let store = store_with(&[&[Some("1"), None, Some(long.as_str()), Some("")], &[Some("2"), Some("b"), None, Some("z")]]);
        assert_eq!(store.cell(0, 0), Some(Some(b"1".to_vec())));
        assert_eq!(store.cell(0, 1), Some(None));
        assert_eq!(store.cell(0, 2).unwrap().unwrap().len(), 70_000);
        assert_eq!(store.cell(0, 3), Some(Some(Vec::new())));
        assert_eq!(store.cell(1, 3), Some(Some(b"z".to_vec())));
        assert_eq!(store.cell(1, 4), None, "no such column");
        assert_eq!(store.cell(2, 0), None, "no such row");
        assert_eq!(store.row(1).unwrap().len(), 4);
        assert_eq!(store.row(2), None);
    }

    #[test]
    fn status_reports_completion_and_error() {
        let store = store_with(&[&[Some("a")]]);
        assert!(!store.status().done);
        store.finish(Some("connection lost".into()));
        let status = store.status();
        assert!(status.done);
        assert_eq!(status.error.as_deref(), Some("connection lost"));
        assert_eq!(status.rows, 1);
    }

    fn people() -> ResultStore {
        let store = store_with(&[
            &[Some("10"), Some("bob")],
            &[Some("9"), Some("Ann")],
            &[Some("100"), None],
            &[None, Some("carol")],
            &[Some("9"), Some("ann")],
        ]);
        store.set_columns(vec![
            ColumnInfo::plain("id", ColumnKind::Number),
            ColumnInfo::plain("name", ColumnKind::Text),
        ]);
        store
    }

    #[test]
    fn numeric_columns_sort_by_value_with_nulls_first() {
        let store = people();
        store.set_view(None, Some((0, false)));
        assert_eq!(
            column(&store, 0),
            [None, Some("9".into()), Some("9".into()), Some("10".into()), Some("100".into())]
        );
        // Equal keys keep their arrival order.
        assert_eq!(column(&store, 1)[1..3], [Some("Ann".to_string()), Some("ann".to_string())]);

        store.set_view(None, Some((0, true)));
        assert_eq!(column(&store, 0)[0], Some("100".into()));
        assert_eq!(column(&store, 0)[4], None);
    }

    #[test]
    fn text_columns_sort_ignoring_ascii_case() {
        let store = people();
        store.set_view(None, Some((1, false)));
        assert_eq!(
            column(&store, 1),
            [None, Some("Ann".into()), Some("ann".into()), Some("bob".into()), Some("carol".into())]
        );
    }

    #[test]
    fn filter_matches_any_cell_ignoring_case_and_combines_with_sort() {
        let store = people();
        assert_eq!(store.set_view(Some("ANN"), None), 2);
        assert_eq!(column(&store, 1), [Some("Ann".into()), Some("ann".into())]);
        assert_eq!(store.status().total_rows, 5);

        assert_eq!(store.set_view(Some("1"), Some((0, true))), 2);
        assert_eq!(column(&store, 0), [Some("100".into()), Some("10".into())]);

        assert_eq!(store.set_view(Some("nobody"), None), 0);
        assert_eq!(store.window(0, 10), 0u32.to_le_bytes());
    }

    #[test]
    fn window_and_row_follow_the_view_and_clearing_it_restores_the_original() {
        let store = people();
        store.set_view(None, Some((1, true)));
        let window = store.window(0, 1);
        assert_eq!(&window[4..], &[0xFB, 5, b'c', b'a', b'r', b'o', b'l'], "carol sorts last, so first when descending");
        assert_eq!(store.row(0).unwrap()[1], Some(b"carol".to_vec()));

        assert_eq!(store.set_view(Some("  "), None), 5, "a blank filter is no filter");
        assert_eq!(column(&store, 1)[0], Some("bob".into()));
    }
}
