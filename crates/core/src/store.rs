//! In-memory storage for a query result.
//!
//! The result lives here, not in the user interface. The UI asks for windows
//! of rows as the user scrolls. Rows are kept in the same compact form the
//! server sent them: one length-prefixed byte string per cell, back to back in
//! a single buffer, so a window of rows is one contiguous slice.
//!
//! Cell encoding (the MySQL length-encoded string):
//! - `0xFB`                      NULL
//! - `n` where n < 251, n bytes  short value
//! - `0xFC`, u16 length, bytes
//! - `0xFD`, u24 length, bytes
//! - `0xFE`, u64 length, bytes

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ColumnInfo {
    pub name: String,
    pub kind: ColumnKind,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoreStatus {
    pub columns: Vec<ColumnInfo>,
    pub rows: u64,
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
    done: bool,
    error: Option<String>,
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

    /// The full value of one cell: `None` if the row or column does not
    /// exist, `Some(None)` for SQL NULL.
    pub fn cell(&self, row: u64, column: usize) -> Option<Option<Vec<u8>>> {
        let inner = self.inner.read().unwrap();
        let start = *inner.offsets.get(row as usize)? as usize;
        let end = *inner.offsets.get(row as usize + 1)? as usize;
        let mut rest = &inner.data[start..end];
        for index in 0.. {
            let (&first, tail) = rest.split_first()?;
            let (len, header) = match first {
                0xFB => {
                    if index == column {
                        return Some(None);
                    }
                    rest = tail;
                    continue;
                }
                0xFC => (u16::from_le_bytes(tail.get(..2)?.try_into().ok()?) as usize, 2),
                0xFD => {
                    let b = tail.get(..3)?;
                    (u32::from_le_bytes([b[0], b[1], b[2], 0]) as usize, 3)
                }
                0xFE => (u64::from_le_bytes(tail.get(..8)?.try_into().ok()?) as usize, 8),
                n => (n as usize, 0),
            };
            let value = tail.get(header..header + len)?;
            if index == column {
                return Some(Some(value.to_vec()));
            }
            rest = &tail[header + len..];
        }
        None
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
            rows: inner.offsets.len() as u64 - 1,
            bytes: inner.data.len() as u64,
            done: inner.done,
            truncated: inner.truncated,
            error: inner.error.clone(),
        }
    }

    /// Returns up to `count` rows starting at row `offset`, ready to send to
    /// the UI: a little-endian `u32` row count, then the encoded rows.
    pub fn window(&self, offset: u64, count: u32) -> Vec<u8> {
        let inner = self.inner.read().unwrap();
        let total = inner.offsets.len() as u64 - 1;
        let first = offset.min(total);
        let last = first.saturating_add(count as u64).min(total);
        let bytes = &inner.data[inner.offsets[first as usize] as usize..inner.offsets[last as usize] as usize];
        let mut out = Vec::with_capacity(4 + bytes.len());
        out.extend_from_slice(&((last - first) as u32).to_le_bytes());
        out.extend_from_slice(bytes);
        out
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
}
