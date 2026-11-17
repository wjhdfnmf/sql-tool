//! In-memory storage for a query result.
//!
//! The result lives here, not in the user interface. The UI asks for windows
//! of rows as the user scrolls. Rows are kept in the same compact form the
//! server sent them: one length-prefixed byte string per cell, back to back in
//! a single buffer, so a window of rows is one contiguous slice.
//!
//! Rows are held in chunks. Up to a memory budget the chunks live in RAM;
//! past it, each further chunk is written to a private temporary file and
//! read back through a memory map, so a very large result costs disk space
//! instead of memory. The file is anonymous and disappears when the result
//! is closed. Its contents are not encrypted.
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
use std::io::{Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
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

    /// Bytes held by the batch, so a reader can flush before it grows large.
    pub fn bytes(&self) -> usize {
        self.data.len()
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

/// How much result data is kept in RAM before spilling to disk, unless the
/// settings or `SQLTOOL_RESULT_MEMORY_MB` say otherwise.
const DEFAULT_MEMORY_BUDGET: u64 = 512 * 1024 * 1024;

/// The budget in force. It applies to each result, and also to all open
/// results together: ten tabs of large results must not use ten times the
/// memory one is allowed.
static MEMORY_BUDGET: AtomicU64 = AtomicU64::new(DEFAULT_MEMORY_BUDGET);
/// Result data held in memory right now, across every open result.
static MEMORY_IN_USE: AtomicU64 = AtomicU64::new(0);

/// Sets how much result data may be held in memory before more goes to disk.
/// Applies to results started from now on, and to the shared total at once.
pub fn set_memory_budget(bytes: u64) {
    MEMORY_BUDGET.store(bytes.max(1024 * 1024), Relaxed);
}

/// Result data held in memory across all open results.
pub fn memory_in_use() -> u64 {
    MEMORY_IN_USE.load(Relaxed)
}

/// Counts memory held on a result's behalf outside its store (the columns a
/// chart keeps as numbers) against the shared budget, so results that arrive
/// later go to disk sooner instead of the total growing unchecked.
pub(crate) fn charge_memory(bytes: u64) {
    MEMORY_IN_USE.fetch_add(bytes, Relaxed);
}

/// Gives back memory counted with [`charge_memory`].
pub(crate) fn release_memory(bytes: u64) {
    MEMORY_IN_USE.fetch_sub(bytes, Relaxed);
}

const DEFAULT_CHUNK_SIZE: usize = 16 * 1024 * 1024;
/// Memory maps must start on a multiple of this (the strictest platform, Windows).
const MAP_ALIGNMENT: u64 = 64 * 1024;

enum Chunk {
    Memory(Vec<u8>),
    Disk(memmap2::Mmap),
}

impl Chunk {
    fn bytes(&self) -> &[u8] {
        match self {
            Chunk::Memory(bytes) => bytes,
            Chunk::Disk(map) => map,
        }
    }
}

/// Where one row's bytes are.
#[derive(Clone, Copy)]
struct RowLocation {
    chunk: u32,
    start: u32,
    end: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoreStatus {
    pub columns: Vec<ColumnInfo>,
    /// Rows currently visible: all of them, or those that pass the filter.
    pub rows: u64,
    /// Rows in the result regardless of any filter.
    pub total_rows: u64,
    pub bytes: u64,
    /// How many of those bytes are on disk instead of in memory.
    pub spilled_bytes: u64,
    /// True once the query has finished, successfully or not.
    pub done: bool,
    /// True when the row limit stopped the result before the server was done.
    pub truncated: bool,
    pub error: Option<String>,
}

struct Inner {
    columns: Vec<ColumnInfo>,
    truncated: bool,
    chunks: Vec<Chunk>,
    rows: Vec<RowLocation>,
    memory_bytes: u64,
    disk_bytes: u64,
    memory_budget: u64,
    chunk_size: usize,
    /// Created on first use. Anonymous: it has no name and is removed by the
    /// operating system when it is closed.
    spill_file: Option<std::fs::File>,
    /// Set if writing to disk failed; from then on everything stays in memory.
    spill_failed: bool,
    /// Row numbers in display order while a sort or filter is active.
    view: Option<Vec<u32>>,
    done: bool,
    error: Option<String>,
}

impl Inner {
    fn total(&self) -> u64 {
        self.rows.len() as u64
    }

    fn visible(&self) -> u64 {
        self.view.as_ref().map_or(self.total(), |v| v.len() as u64)
    }

    fn raw_row(&self, row: usize) -> &[u8] {
        let at = self.rows[row];
        &self.chunks[at.chunk as usize].bytes()[at.start as usize..at.end as usize]
    }

    fn push_row(&mut self, row: &[u8]) {
        let fits = matches!(self.chunks.last(), Some(Chunk::Memory(open)) if open.len() + row.len() <= self.chunk_size);
        if !fits {
            self.seal_last_chunk();
            // A row larger than a chunk gets a chunk of its own.
            self.chunks.push(Chunk::Memory(Vec::with_capacity(row.len().max(self.chunk_size.min(1 << 20)))));
        }
        let chunk = self.chunks.len() - 1;
        let Some(Chunk::Memory(open)) = self.chunks.last_mut() else {
            unreachable!("the last chunk was just made writable");
        };
        let start = open.len();
        open.extend_from_slice(row);
        self.memory_bytes += row.len() as u64;
        MEMORY_IN_USE.fetch_add(row.len() as u64, Relaxed);
        self.rows.push(RowLocation {
            chunk: chunk as u32,
            start: start as u32,
            end: (start + row.len()) as u32,
        });
    }

    /// Called when the last chunk will not be written to again. If memory is
    /// over budget, the chunk moves to disk.
    fn seal_last_chunk(&mut self) {
        let own = self.memory_bytes > self.memory_budget;
        let shared = MEMORY_IN_USE.load(Relaxed) > MEMORY_BUDGET.load(Relaxed);
        if !(own || shared) || self.spill_failed {
            return;
        }
        let Some(Chunk::Memory(bytes)) = self.chunks.last() else {
            return;
        };
        let len = bytes.len() as u64;
        match write_to_disk(&mut self.spill_file, bytes) {
            Ok(map) => {
                self.memory_bytes -= len;
                MEMORY_IN_USE.fetch_sub(len, Relaxed);
                self.disk_bytes += len;
                *self.chunks.last_mut().expect("checked above") = Chunk::Disk(map);
            }
            // Out of disk space, or no temporary directory: keep working in memory.
            Err(_) => self.spill_failed = true,
        }
    }


    /// The stored row behind a visible row number.
    fn resolve(&self, visible_row: u64) -> Option<usize> {
        match &self.view {
            Some(view) => view.get(visible_row as usize).map(|r| *r as usize),
            None => (visible_row < self.total()).then_some(visible_row as usize),
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        // Closing a result gives its memory back to the shared budget.
        MEMORY_IN_USE.fetch_sub(self.memory_bytes, Relaxed);
    }
}

/// Appends `bytes` to the spill file (creating it on first use) and maps
/// them back in.
fn write_to_disk(spill_file: &mut Option<std::fs::File>, bytes: &[u8]) -> std::io::Result<memmap2::Mmap> {
    if spill_file.is_none() {
        *spill_file = Some(tempfile::tempfile()?);
    }
    let file = spill_file.as_mut().expect("just created");
    let end = file.seek(SeekFrom::End(0))?;
    let offset = end.div_ceil(MAP_ALIGNMENT) * MAP_ALIGNMENT;
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(bytes)?;
    file.flush()?;
    // SAFETY: the file is anonymous and private to this process, and this
    // region is never written again, so the mapped bytes cannot change.
    unsafe { memmap2::MmapOptions::new().offset(offset).len(bytes.len()).map(&*file) }
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
        let budget = std::env::var("SQLTOOL_RESULT_MEMORY_MB")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map_or(MEMORY_BUDGET.load(Relaxed), |mb| mb * 1024 * 1024);
        Self::with_limits(budget, DEFAULT_CHUNK_SIZE)
    }

    /// A store that keeps about `memory_budget` bytes in RAM and spills the
    /// rest to disk, `chunk_size` bytes at a time.
    pub fn with_limits(memory_budget: u64, chunk_size: usize) -> Self {
        Self {
            inner: RwLock::new(Inner {
                columns: Vec::new(),
                truncated: false,
                chunks: Vec::new(),
                rows: Vec::new(),
                memory_bytes: 0,
                disk_bytes: 0,
                memory_budget,
                chunk_size: chunk_size.max(1),
                spill_file: None,
                spill_failed: false,
                view: None,
                done: false,
                error: None,
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
        let mut start = 0usize;
        for end in &batch.ends {
            inner.push_row(&batch.data[start..*end as usize]);
            start = *end as usize;
        }
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
            bytes: inner.memory_bytes + inner.disk_bytes,
            spilled_bytes: inner.disk_bytes,
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
        for visible_row in first..last {
            let row = inner.resolve(visible_row).expect("within the visible rows");
            out.extend_from_slice(inner.raw_row(row));
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

    /// Calls `visit` with the cells of every stored row, in the order they
    /// arrived, whatever the grid is sorted or filtered by. Returns how many.
    pub(crate) fn for_each_stored_row(&self, mut visit: impl FnMut(Cells<'_>)) -> usize {
        let inner = self.inner.read().unwrap();
        for row in 0..inner.rows.len() {
            visit(Cells(inner.raw_row(row)));
        }
        inner.rows.len()
    }

    /// Calls `visit` with the cells of visible rows `first..=last`.
    pub(crate) fn for_each_row_in<E>(
        &self,
        first: u64,
        last: u64,
        mut visit: impl FnMut(Cells<'_>) -> Result<(), E>,
    ) -> Result<(), E> {
        let inner = self.inner.read().unwrap();
        for index in first..=last.min(inner.visible().saturating_sub(1)) {
            let Some(row) = inner.resolve(index) else { break };
            visit(Cells(inner.raw_row(row)))?;
        }
        Ok(())
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
            // Each row's sort key is read out once, up front. Comparing by
            // walking the row again on every comparison is far slower.
            enum Key<'a> {
                Null,
                Number(f64),
                Text(&'a [u8]),
            }
            let numeric = inner.columns.get(column).is_some_and(|c| c.kind == ColumnKind::Number);
            let mut keyed: Vec<(Key<'_>, u32)> = rows
                .iter()
                .map(|&row| {
                    let key = match Cells(inner.raw_row(row as usize)).nth(column).flatten() {
                        None => Key::Null,
                        Some(bytes) => std::str::from_utf8(bytes)
                            .ok()
                            .filter(|_| numeric)
                            .and_then(|s| s.trim().parse::<f64>().ok())
                            .map_or(Key::Text(bytes), Key::Number),
                    };
                    (key, row)
                })
                .collect();
            // Stable, so rows that compare equal keep the order they arrived in.
            keyed.sort_by(|(a, _), (b, _)| {
                let ordering = match (a, b) {
                    (Key::Null, Key::Null) => Ordering::Equal,
                    (Key::Null, _) => Ordering::Less,
                    (_, Key::Null) => Ordering::Greater,
                    (Key::Number(x), Key::Number(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
                    (Key::Number(_), Key::Text(_)) => Ordering::Less,
                    (Key::Text(_), Key::Number(_)) => Ordering::Greater,
                    (Key::Text(x), Key::Text(y)) => x
                        .iter()
                        .map(u8::to_ascii_lowercase)
                        .cmp(y.iter().map(u8::to_ascii_lowercase))
                        .then_with(|| x.cmp(y)),
                };
                if descending { ordering.reverse() } else { ordering }
            });
            rows = keyed.into_iter().map(|(_, row)| row).collect();
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

    /// A store small enough that a few hundred rows spill to disk.
    fn spilling_store(rows: usize) -> ResultStore {
        let store = ResultStore::with_limits(2_000, 1_000);
        store.set_columns(vec![ColumnInfo::plain("id", ColumnKind::Number), ColumnInfo::plain("text", ColumnKind::Text)]);
        let mut batch = RowBatch::default();
        for i in 0..rows {
            let id = i.to_string();
            let text = format!("row {i} {}", "x".repeat(i % 40));
            batch.push_row([Some(id.as_bytes()), Some(text.as_bytes())]);
            if i % 7 == 0 {
                store.append(&mut batch);
            }
        }
        store.append(&mut batch);
        store
    }

    #[test]
    fn rows_past_the_memory_budget_go_to_disk_and_read_back_unchanged() {
        let store = spilling_store(500);
        let status = store.status();
        assert_eq!(status.rows, 500);
        assert!(status.spilled_bytes > 0, "nothing was spilled");
        assert!(status.bytes - status.spilled_bytes <= 3_000, "memory use is bounded near the budget");
        for i in [0u64, 1, 57, 249, 499] {
            assert_eq!(store.cell(i, 0), Some(Some(i.to_string().into_bytes())));
            let expected = format!("row {i} {}", "x".repeat(i as usize % 40));
            assert_eq!(store.cell(i, 1), Some(Some(expected.into_bytes())));
        }
    }

    #[test]
    fn windows_sorting_and_filtering_work_across_memory_and_disk() {
        let store = spilling_store(500);
        let window = store.window(498, 10);
        assert_eq!(&window[..4], &2u32.to_le_bytes());
        assert_eq!(&window[4..8], b"\x03498");

        assert_eq!(store.set_view(None, Some((0, true))), 500);
        assert_eq!(store.cell(0, 0), Some(Some(b"499".to_vec())), "highest id first");
        assert_eq!(store.cell(499, 0), Some(Some(b"0".to_vec())));

        assert_eq!(store.set_view(Some("row 49"), None), 11, "row 49 and rows 490 to 499");
        assert_eq!(store.cell(1, 0), Some(Some(b"490".to_vec())));
    }

    #[test]
    fn a_row_larger_than_a_chunk_is_stored_whole() {
        let store = ResultStore::with_limits(100, 64);
        let big = "y".repeat(10_000);
        let mut batch = RowBatch::default();
        batch.push_row([Some(&b"before"[..])]);
        batch.push_row([Some(big.as_bytes())]);
        batch.push_row([Some(&b"after"[..])]);
        store.append(&mut batch);
        assert_eq!(store.cell(0, 0), Some(Some(b"before".to_vec())));
        assert_eq!(store.cell(1, 0).unwrap().unwrap().len(), 10_000);
        assert_eq!(store.cell(2, 0), Some(Some(b"after".to_vec())));
        assert!(store.status().spilled_bytes >= 10_000);
    }

    #[test]
    fn a_store_within_budget_never_touches_disk() {
        let store = store_with(&[&[Some("a")], &[Some("b")]]);
        assert_eq!(store.status().spilled_bytes, 0);
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
