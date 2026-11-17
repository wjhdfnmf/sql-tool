//! Summaries of a result, for drawing charts.
//!
//! A result can hold millions of rows and the user interface holds none of
//! them, so a chart is not drawn from rows. The interface says what it wants
//! to see (which columns on which axis, which kind of chart, which filters)
//! and gets back only what is drawn: one number per bar, one count per bin,
//! or the points of a scatter plot up to a limit, past which it gets a grid
//! of counts or a sample instead.
//!
//! Cells are stored as text. The first time a column is used it is read out
//! once into numbers (or into category codes) and kept, so that changing a
//! filter or zooming does not parse millions of cells again. Those kept
//! columns count against the same memory budget as the results themselves
//! and are released when the chart is closed.
//!
//! A chart always works on every row of the result, in the order the rows
//! arrived. The grid's own sort and filter do not affect it; it has filters
//! of its own.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::store::{self, ResultStore};

/// Points of a scatter plot sent as they are. Past this the plot becomes a
/// grid of counts, or a sample.
pub const MAX_POINTS: usize = 50_000;
/// Different values of one column that are told apart. The rest are lumped together.
const MAX_LABELS: usize = 5_000;
/// Groups a colour column is split into, the last being "Other" when there are
/// more. Eight is as many colours as can be told apart, including by readers
/// who do not see every colour.
pub const MAX_SERIES: usize = 8;
const DEFAULT_CATEGORIES: usize = 40;
const MAX_CATEGORIES: usize = 500;
const MAX_BINS: usize = 500;
/// Time buckets drawn. A finer unit than this allows is made coarser.
const MAX_BUCKETS: i64 = 1_000;
const DENSITY_COLS: usize = 160;
const DENSITY_ROWS: usize = 100;
/// Categories listed for a filter.
const MAX_LISTED: usize = 1_000;
/// Columns kept as numbers before the ones not in use are dropped.
const MAX_KEPT_COLUMNS: usize = 8;
/// Bytes of a value that tell categories apart, and characters of it shown.
const KEY_BYTES: usize = 256;
const LABEL_CHARS: usize = 120;

const MISSING: u32 = u32::MAX;
const OTHER: u32 = u32::MAX - 1;
const DAY_MS: f64 = 86_400_000.0;

pub const NULL_LABEL: &str = "(null)";
pub const OTHER_LABEL: &str = "Other";

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ChartError(pub String);

fn error<T>(message: impl Into<String>) -> Result<T, ChartError> {
    Err(ChartError(message.into()))
}

/// How a column's values are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Numbers on a continuous scale.
    Continuous,
    /// Dates and times, as milliseconds since 1970-01-01 with no time zone.
    Time,
    /// Separate values, each its own category.
    Discrete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AxisRef {
    pub column: usize,
    pub role: Role,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChartKind {
    Bar,
    Histogram,
    Scatter,
}

/// What a bar shows of the Y column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Statistic {
    #[default]
    Count,
    Sum,
    Mean,
    Min,
    Max,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimeBucket {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

/// What a scatter plot of more than [`MAX_POINTS`] points becomes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Large {
    /// A grid of counts.
    #[default]
    Density,
    /// A random sample of the points.
    Sample,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BarSort {
    /// Largest bar first.
    #[default]
    Value,
    /// By category name; numbers in numeric order.
    Label,
    /// In the order the categories first appear in the result.
    Seen,
}

/// Keeps rows by one column. For a continuous or time column, `min` and
/// `max` (either may be left out); for a discrete one, `values`, where
/// `None` stands for NULL and an empty list keeps everything. A row whose
/// value is NULL or unreadable does not pass a range.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    pub column: usize,
    pub role: Role,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub values: Vec<Option<String>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartRequest {
    pub kind: ChartKind,
    #[serde(default)]
    pub x: Option<AxisRef>,
    #[serde(default)]
    pub y: Option<AxisRef>,
    /// A column whose values split the chart into coloured groups.
    #[serde(default)]
    pub color: Option<usize>,
    #[serde(default)]
    pub statistic: Statistic,
    /// About how many bins a continuous X is cut into. Chosen from the data if left out.
    #[serde(default)]
    pub bins: Option<u32>,
    /// The unit a time X is cut into. Chosen from the data if left out.
    #[serde(default)]
    pub bucket: Option<TimeBucket>,
    #[serde(default)]
    pub filters: Vec<Filter>,
    /// Keeps rows whose X is within these bounds: the time range, or the zoom of a scatter plot.
    #[serde(default)]
    pub x_range: Option<[f64; 2]>,
    #[serde(default)]
    pub y_range: Option<[f64; 2]>,
    #[serde(default)]
    pub large: Large,
    #[serde(default)]
    pub sort: BarSort,
    /// Bars shown for a discrete X before the smallest are merged into "Other".
    #[serde(default)]
    pub max_categories: Option<u32>,
}

impl ChartRequest {
    /// A chart of `kind` with nothing on it yet.
    pub fn new(kind: ChartKind) -> Self {
        Self {
            kind,
            x: None,
            y: None,
            color: None,
            statistic: Statistic::default(),
            bins: None,
            bucket: None,
            filters: Vec::new(),
            x_range: None,
            y_range: None,
            large: Large::default(),
            sort: BarSort::default(),
            max_categories: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum XAxis {
    /// One bar per label.
    Categories { labels: Vec<String> },
    /// Bars that each cover `edges[i]..edges[i + 1]`. `bucket` is set for time.
    #[serde(rename_all = "camelCase")]
    Bins { edges: Vec<f64>, time: bool, bucket: Option<TimeBucket> },
    /// Points on a continuous scale.
    Continuous { time: bool },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Series {
    /// The colour group, or `None` when the chart is not split by colour.
    pub name: Option<String>,
    /// One value per category or bin. `None` where there is nothing to show.
    pub values: Vec<Option<f64>>,
    /// Scatter points as x, y, x, y and so on.
    pub points: Vec<f64>,
}

/// How many points fall in each cell of a grid over the plot.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Density {
    pub cols: usize,
    pub rows: usize,
    pub x: [f64; 2],
    pub y: [f64; 2],
    /// Row by row from the lowest Y, `cols` counts per row.
    pub counts: Vec<u32>,
    pub max: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartData {
    /// Rows in the result.
    pub rows_total: u64,
    /// Rows that pass the filters and ranges.
    pub rows_filtered: u64,
    /// Of those, rows with a usable value on every axis in use.
    pub rows_drawn: u64,
    pub x: XAxis,
    pub series: Vec<Series>,
    pub density: Option<Density>,
    /// The points are a sample of `rows_drawn`.
    pub sampled: bool,
    /// The smallest and largest X among rows that pass the filters, ignoring
    /// the X range: what a range slider spans.
    pub x_extent: Option<[f64; 2]>,
    /// The X or Y column holds times of day (`TIME`), not dates.
    pub x_time_only: bool,
    pub y_time_only: bool,
    /// Things the reader should know about what was left out or merged.
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryCount {
    /// `None` is NULL.
    pub label: Option<String>,
    pub rows: u64,
}

/// What a column holds when read in one role, for filter controls.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnSummary {
    pub rows: u64,
    /// NULL, or not readable in this role (text in a number column).
    pub missing: u64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub time_only: bool,
    /// Different values, for a discrete column.
    pub distinct: u64,
    /// There are more different values than are told apart.
    pub overflow: bool,
    /// The most frequent values, most frequent first.
    pub categories: Vec<CategoryCount>,
}

// ---------- columns read out of the store ----------

enum Values {
    /// NaN where the cell is NULL or unreadable.
    Numbers(Vec<f64>),
    Codes { codes: Vec<u32>, labels: Vec<String>, overflow: bool },
}

struct Column {
    values: Values,
    missing: u64,
    min: f64,
    max: f64,
    time_only: bool,
}

impl Column {
    fn bytes(&self) -> u64 {
        match &self.values {
            Values::Numbers(v) => (v.len() * 8) as u64,
            Values::Codes { codes, labels, .. } => {
                (codes.len() * 4 + labels.iter().map(|l| l.len() + 24).sum::<usize>()) as u64
            }
        }
    }

    fn numbers(&self) -> Option<&[f64]> {
        match &self.values {
            Values::Numbers(v) => Some(v),
            Values::Codes { .. } => None,
        }
    }
}

enum Builder {
    Number(Vec<f64>),
    Time { out: Vec<f64>, dates: u64 },
    Category {
        codes: Vec<u32>,
        index: HashMap<Box<[u8]>, u32>,
        labels: Vec<String>,
        overflow: bool,
        /// The previous value and its code: columns often repeat in runs.
        last: Option<(Box<[u8]>, u32)>,
    },
}

impl Builder {
    fn new(role: Role, rows: usize) -> Self {
        match role {
            Role::Continuous => Builder::Number(Vec::with_capacity(rows)),
            Role::Time => Builder::Time { out: Vec::with_capacity(rows), dates: 0 },
            Role::Discrete => Builder::Category {
                codes: Vec::with_capacity(rows),
                index: HashMap::new(),
                labels: Vec::new(),
                overflow: false,
                last: None,
            },
        }
    }

    fn push(&mut self, cell: Option<&[u8]>) {
        match self {
            Builder::Number(out) => out.push(cell.and_then(parse_number).unwrap_or(f64::NAN)),
            Builder::Time { out, dates } => match cell.and_then(parse_time) {
                Some((ms, has_date)) => {
                    *dates += has_date as u64;
                    out.push(ms);
                }
                None => out.push(f64::NAN),
            },
            Builder::Category { codes, index, labels, overflow, last } => {
                let Some(bytes) = cell else {
                    codes.push(MISSING);
                    return;
                };
                let key = &bytes[..bytes.len().min(KEY_BYTES)];
                if let Some((previous, code)) = last {
                    if **previous == *key {
                        codes.push(*code);
                        return;
                    }
                }
                let code = match index.get(key) {
                    Some(code) => *code,
                    None if labels.len() >= MAX_LABELS => {
                        *overflow = true;
                        OTHER
                    }
                    None => {
                        let code = labels.len() as u32;
                        labels.push(label_of(key, bytes.len() > key.len()));
                        index.insert(key.into(), code);
                        code
                    }
                };
                *last = Some((key.into(), code));
                codes.push(code);
            }
        }
    }

    fn finish(self) -> Column {
        match self {
            Builder::Number(out) => numbers_column(out, false),
            Builder::Time { out, dates } => {
                let any = out.iter().any(|v| !v.is_nan());
                numbers_column(out, any && dates == 0)
            }
            Builder::Category { codes, labels, overflow, .. } => Column {
                missing: codes.iter().filter(|c| **c == MISSING).count() as u64,
                values: Values::Codes { codes, labels, overflow },
                min: f64::NAN,
                max: f64::NAN,
                time_only: false,
            },
        }
    }
}

fn numbers_column(values: Vec<f64>, time_only: bool) -> Column {
    let mut missing = 0u64;
    let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
    for v in &values {
        if v.is_nan() {
            missing += 1;
        } else {
            min = min.min(*v);
            max = max.max(*v);
        }
    }
    if min > max {
        (min, max) = (f64::NAN, f64::NAN);
    }
    Column { values: Values::Numbers(values), missing, min, max, time_only }
}

fn label_of(key: &[u8], cut: bool) -> String {
    let text = String::from_utf8_lossy(key);
    let mut label: String = text.chars().take(LABEL_CHARS).collect();
    if cut || text.chars().count() > LABEL_CHARS {
        label.push('…');
    }
    label
}

fn parse_number(cell: &[u8]) -> Option<f64> {
    std::str::from_utf8(cell).ok()?.trim().parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Reads a date, a date and time, or a time of day as MySQL writes them.
/// Returns milliseconds (since 1970-01-01 for a date, since midnight for a
/// time) and whether there was a date. Zero dates (`0000-00-00`) are not
/// dates and give `None`.
fn parse_time(cell: &[u8]) -> Option<(f64, bool)> {
    let text = cell.trim_ascii();
    if text.len() >= 10 && text[4] == b'-' && text[7] == b'-' {
        let (year, month, day) = (digits(&text[..4])?, digits(&text[5..7])?, digits(&text[8..10])?);
        if year == 0 || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        let mut ms = days_from_civil(year, month, day) as f64 * DAY_MS;
        if let Some((separator, clock)) = text[10..].split_first() {
            if *separator != b' ' && *separator != b'T' {
                return None;
            }
            ms += parse_clock(clock)?;
        }
        return Some((ms, true));
    }
    let (negative, clock) = match text.split_first()? {
        (b'-', rest) => (true, rest),
        _ => (false, text),
    };
    let ms = parse_clock(clock)?;
    Some((if negative { -ms } else { ms }, false))
}

/// `hh:mm`, `hh:mm:ss` or `hh:mm:ss.ffffff`, as milliseconds. Hours may exceed 23, as in a `TIME`.
fn parse_clock(text: &[u8]) -> Option<f64> {
    let mut parts = text.splitn(3, |b| *b == b':');
    let hours = digits(parts.next()?)?;
    let minutes = digits(parts.next()?)?;
    let seconds = match parts.next() {
        None => 0.0,
        Some(s) if s.first().is_some_and(u8::is_ascii_digit) => std::str::from_utf8(s).ok()?.parse::<f64>().ok()?,
        Some(_) => return None,
    };
    if minutes > 59 || !(0.0..61.0).contains(&seconds) {
        return None;
    }
    Some((hours * 3600 + minutes * 60) as f64 * 1000.0 + seconds * 1000.0)
}

fn digits(text: &[u8]) -> Option<i64> {
    if text.is_empty() || text.len() > 4 || !text.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(text.iter().fold(0i64, |n, d| n * 10 + (d - b'0') as i64))
}

/// Days since 1970-01-01 of a date in the Gregorian calendar.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The year, month and day of a number of days since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (year_of_era + era * 400 + (month <= 2) as i64, month, day)
}

impl TimeBucket {
    const ALL: [TimeBucket; 8] = [
        TimeBucket::Second,
        TimeBucket::Minute,
        TimeBucket::Hour,
        TimeBucket::Day,
        TimeBucket::Week,
        TimeBucket::Month,
        TimeBucket::Quarter,
        TimeBucket::Year,
    ];

    /// Which bucket a time falls in. Buckets are numbered so that the next one is always one more.
    fn index(self, ms: f64) -> i64 {
        let whole = |size: f64| (ms / size).floor() as i64;
        match self {
            TimeBucket::Second => whole(1_000.0),
            TimeBucket::Minute => whole(60_000.0),
            TimeBucket::Hour => whole(3_600_000.0),
            TimeBucket::Day => whole(DAY_MS),
            // Weeks start on Monday. Day 0 was a Thursday.
            TimeBucket::Week => (whole(DAY_MS) + 3).div_euclid(7),
            TimeBucket::Month | TimeBucket::Quarter | TimeBucket::Year => {
                let (year, month, _) = civil_from_days(whole(DAY_MS));
                match self {
                    TimeBucket::Month => year * 12 + month - 1,
                    TimeBucket::Quarter => (year * 12 + month - 1).div_euclid(3),
                    _ => year,
                }
            }
        }
    }

    /// When a bucket begins.
    fn start(self, index: i64) -> f64 {
        match self {
            TimeBucket::Second => index as f64 * 1_000.0,
            TimeBucket::Minute => index as f64 * 60_000.0,
            TimeBucket::Hour => index as f64 * 3_600_000.0,
            TimeBucket::Day => index as f64 * DAY_MS,
            TimeBucket::Week => (index * 7 - 3) as f64 * DAY_MS,
            TimeBucket::Month => days_from_civil(index.div_euclid(12), index.rem_euclid(12) + 1, 1) as f64 * DAY_MS,
            TimeBucket::Quarter => TimeBucket::Month.start(index * 3),
            TimeBucket::Year => days_from_civil(index, 1, 1) as f64 * DAY_MS,
        }
    }
}

// ---------- the dataset ----------

struct Kept {
    /// Rows in the result when the first column was read.
    rows: Option<usize>,
    columns: HashMap<(usize, Role), Arc<Column>>,
    bytes: u64,
}

/// A finished result prepared for charting: the store, and the columns read
/// out of it so far.
pub struct Dataset {
    store: Arc<ResultStore>,
    kept: Mutex<Kept>,
}

impl Drop for Dataset {
    fn drop(&mut self) {
        let kept = self.kept.get_mut().unwrap_or_else(|e| e.into_inner());
        store::release_memory(kept.bytes);
    }
}

struct Loaded {
    rows: usize,
    columns: HashMap<(usize, Role), Arc<Column>>,
}

impl Loaded {
    fn get(&self, column: usize, role: Role) -> &Column {
        &self.columns[&(column, role)]
    }
}

impl Dataset {
    pub fn new(store: Arc<ResultStore>) -> Self {
        Self { store, kept: Mutex::new(Kept { rows: None, columns: HashMap::new(), bytes: 0 }) }
    }

    /// Memory held by the columns read out so far.
    pub fn kept_bytes(&self) -> u64 {
        self.kept.lock().unwrap_or_else(|e| e.into_inner()).bytes
    }

    /// Reads the wanted columns out of the store, in one pass over the rows
    /// for all that are not already kept.
    fn load(&self, wanted: &[(usize, Role)]) -> Result<Loaded, ChartError> {
        let status = self.store.status();
        if !status.done {
            return error("The result is still loading. A chart can be drawn once it has finished.");
        }
        for (column, _) in wanted {
            if *column >= status.columns.len() {
                return error("The result no longer has that column.");
            }
        }
        let mut kept = self.kept.lock().unwrap_or_else(|e| e.into_inner());
        let mut missing: Vec<(usize, Role)> = wanted.iter().copied().filter(|key| !kept.columns.contains_key(key)).collect();
        missing.sort_by_key(|(column, role)| (*column, *role as u8));
        missing.dedup();

        if !missing.is_empty() {
            // Make room first: columns no longer on the chart are the ones to let go.
            if kept.columns.len() + missing.len() > MAX_KEPT_COLUMNS {
                let before = kept.bytes;
                kept.columns.retain(|key, _| wanted.contains(key));
                kept.bytes = kept.columns.values().map(|c| c.bytes()).sum();
                store::release_memory(before - kept.bytes);
            }
            let expected = status.total_rows as usize;
            let mut builders: Vec<(usize, Builder)> =
                missing.iter().map(|(column, role)| (*column, Builder::new(*role, expected))).collect();
            let rows = self.store.for_each_stored_row(|cells| {
                let mut next = 0;
                for (index, cell) in cells.enumerate() {
                    while next < builders.len() && builders[next].0 == index {
                        builders[next].1.push(cell);
                        next += 1;
                    }
                    if next == builders.len() {
                        break;
                    }
                }
                // A row shorter than the column list has nothing there.
                for (_, builder) in &mut builders[next..] {
                    builder.push(None);
                }
            });
            if *kept.rows.get_or_insert(rows) != rows {
                return error("The result changed while the chart was open. Close the chart and open it again.");
            }
            for (key, (_, builder)) in missing.iter().zip(builders) {
                let column = builder.finish();
                let bytes = column.bytes();
                kept.bytes += bytes;
                store::charge_memory(bytes);
                kept.columns.insert(*key, Arc::new(column));
            }
        }
        let rows = *kept.rows.get_or_insert(status.total_rows as usize);
        Ok(Loaded {
            rows,
            columns: wanted.iter().map(|key| (*key, kept.columns[key].clone())).collect(),
        })
    }

    /// What one column holds, read in `role`.
    pub fn summary(&self, column: usize, role: Role) -> Result<ColumnSummary, ChartError> {
        let loaded = self.load(&[(column, role)])?;
        let column = loaded.get(column, role);
        let number = |v: f64| (!v.is_nan()).then_some(v);
        let mut summary = ColumnSummary {
            rows: loaded.rows as u64,
            missing: column.missing,
            min: number(column.min),
            max: number(column.max),
            time_only: column.time_only,
            distinct: 0,
            overflow: false,
            categories: Vec::new(),
        };
        if let Values::Codes { codes, labels, overflow } = &column.values {
            let mut counts = vec![0u64; labels.len()];
            for code in codes {
                if let Some(count) = counts.get_mut(*code as usize) {
                    *count += 1;
                }
            }
            let mut listed: Vec<CategoryCount> =
                labels.iter().zip(counts).map(|(label, rows)| CategoryCount { label: Some(label.clone()), rows }).collect();
            if column.missing > 0 {
                listed.push(CategoryCount { label: None, rows: column.missing });
            }
            listed.sort_by(|a, b| b.rows.cmp(&a.rows).then_with(|| natural(a.label.as_deref(), b.label.as_deref())));
            listed.truncate(MAX_LISTED);
            summary.distinct = labels.len() as u64;
            summary.overflow = *overflow;
            summary.categories = listed;
        }
        Ok(summary)
    }

    /// Works out what a chart shows.
    pub fn chart(&self, request: &ChartRequest) -> Result<ChartData, ChartError> {
        let mut wanted: Vec<(usize, Role)> = Vec::new();
        wanted.extend(request.x.map(|a| (a.column, a.role)));
        wanted.extend(request.y.map(|a| (a.column, a.role)));
        wanted.extend(request.color.map(|column| (column, Role::Discrete)));
        wanted.extend(request.filters.iter().map(|f| (f.column, f.role)));
        let loaded = self.load(&wanted)?;
        let rows = loaded.rows;
        let x = request.x.map(|a| loaded.get(a.column, a.role));
        let y = request.y.map(|a| loaded.get(a.column, a.role));
        let color = request.color.map(|column| loaded.get(column, Role::Discrete));

        let mut mask = vec![true; rows];
        for filter in &request.filters {
            apply_filter(&mut mask, loaded.get(filter.column, filter.role), filter);
        }
        let x_numbers = x.and_then(Column::numbers);
        let x_extent = x_numbers.and_then(|values| extent(values, &mask));
        if let (Some(values), Some(range)) = (x_numbers, request.x_range) {
            keep_within(&mut mask, values, range);
        }
        if let (ChartKind::Scatter, Some(values), Some(range)) = (request.kind, y.and_then(Column::numbers), request.y_range) {
            keep_within(&mut mask, values, range);
        }

        let mut data = ChartData {
            rows_total: rows as u64,
            rows_filtered: mask.iter().filter(|m| **m).count() as u64,
            rows_drawn: 0,
            x: XAxis::Categories { labels: Vec::new() },
            series: Vec::new(),
            density: None,
            sampled: false,
            x_extent,
            x_time_only: x.is_some_and(|c| c.time_only),
            y_time_only: y.is_some_and(|c| c.time_only),
            notes: Vec::new(),
        };
        let groups = SeriesMap::new(color, &mask);
        if groups.merged {
            data.notes.push(format!(
                "The colour column has more than {MAX_SERIES} values; the {} with the most rows are shown and the rest are under {OTHER_LABEL}.",
                MAX_SERIES - 1
            ));
        }

        match request.kind {
            ChartKind::Scatter => {
                let (Some(xv), Some(yv)) = (x_numbers, y.and_then(Column::numbers)) else {
                    return error("A scatter plot needs a number or time column on X and on Y.");
                };
                data.x = XAxis::Continuous { time: request.x.is_some_and(|a| a.role == Role::Time) };
                scatter(&mut data, xv, yv, &mask, &groups, request.large);
            }
            ChartKind::Histogram => {
                let Some(xv) = x_numbers else {
                    return error("A histogram needs a number or time column on X.");
                };
                let time = request.x.is_some_and(|a| a.role == Role::Time);
                binned(&mut data, xv, time, request, &mask, YValues::None, Statistic::Count, &groups);
            }
            ChartKind::Bar => {
                let (yv, statistic) = match y.map(|c| &c.values) {
                    None => (YValues::None, Statistic::Count),
                    Some(Values::Numbers(values)) => (YValues::Numbers(values), request.statistic),
                    // Categories cannot be added up; the bar counts the rows that have one.
                    Some(Values::Codes { codes, .. }) => (YValues::Present(codes), Statistic::Count),
                };
                match x.map(|c| &c.values) {
                    Some(Values::Numbers(xv)) => {
                        let time = request.x.is_some_and(|a| a.role == Role::Time);
                        binned(&mut data, xv, time, request, &mask, yv, statistic, &groups);
                    }
                    Some(Values::Codes { codes, labels, .. }) => {
                        by_category(&mut data, Some(codes), labels, request, &mask, yv, statistic, &groups);
                    }
                    None => by_category(&mut data, None, &[], request, &mask, yv, statistic, &groups),
                }
            }
        }
        Ok(data)
    }
}

// ---------- filters ----------

fn apply_filter(mask: &mut [bool], column: &Column, filter: &Filter) {
    match &column.values {
        Values::Numbers(values) => {
            if filter.min.is_none() && filter.max.is_none() {
                return;
            }
            let (min, max) = (filter.min.unwrap_or(f64::NEG_INFINITY), filter.max.unwrap_or(f64::INFINITY));
            keep_within(mask, values, [min, max]);
        }
        Values::Codes { codes, labels, .. } => {
            if filter.values.is_empty() {
                return;
            }
            let wanted: std::collections::HashSet<&str> = filter.values.iter().flatten().map(String::as_str).collect();
            let allowed: Vec<bool> = labels.iter().map(|label| wanted.contains(label.as_str())).collect();
            let nulls = filter.values.iter().any(Option::is_none);
            for (keep, code) in mask.iter_mut().zip(codes) {
                if *keep {
                    *keep = match *code {
                        MISSING => nulls,
                        // Values past the limit have no name to be chosen by.
                        OTHER => false,
                        code => allowed[code as usize],
                    };
                }
            }
        }
    }
}

/// Clears the mask for rows whose value is outside the bounds, or missing.
fn keep_within(mask: &mut [bool], values: &[f64], [min, max]: [f64; 2]) {
    for (keep, value) in mask.iter_mut().zip(values) {
        // A NaN fails both comparisons, so missing values do not pass.
        *keep = *keep && *value >= min && *value <= max;
    }
}

fn extent(values: &[f64], mask: &[bool]) -> Option<[f64; 2]> {
    let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
    for (value, keep) in values.iter().zip(mask) {
        if *keep && !value.is_nan() {
            min = min.min(*value);
            max = max.max(*value);
        }
    }
    (min <= max).then_some([min, max])
}

// ---------- colour groups ----------

/// Which series each row belongs to.
struct SeriesMap<'a> {
    codes: Option<&'a [u32]>,
    lookup: Vec<u8>,
    missing: u8,
    other: u8,
    names: Vec<Option<String>>,
    /// Some groups were merged into "Other".
    merged: bool,
}

impl<'a> SeriesMap<'a> {
    fn new(color: Option<&'a Column>, mask: &[bool]) -> Self {
        let Some(Values::Codes { codes, labels, .. }) = color.map(|c| &c.values) else {
            return Self { codes: None, lookup: Vec::new(), missing: 0, other: 0, names: vec![None], merged: false };
        };
        // Rows per group among those on the chart: labels, then NULL, then the lumped rest.
        let (null_at, rest_at) = (labels.len(), labels.len() + 1);
        let mut counts = vec![0u64; labels.len() + 2];
        for (code, keep) in codes.iter().zip(mask) {
            if *keep {
                counts[match *code {
                    MISSING => null_at,
                    OTHER => rest_at,
                    code => code as usize,
                }] += 1;
            }
        }
        let name = |group: usize| if group == null_at { NULL_LABEL } else { labels[group].as_str() };
        let mut present: Vec<usize> = (0..=null_at).filter(|group| counts[*group] > 0).collect();
        let merged = present.len() > MAX_SERIES || (counts[rest_at] > 0 && present.len() == MAX_SERIES);
        let needs_other = merged || counts[rest_at] > 0;
        if merged {
            present.sort_by(|a, b| counts[*b].cmp(&counts[*a]).then(a.cmp(b)));
            present.truncate(MAX_SERIES - 1);
        }
        // In name order, so a group keeps its place and colour as filters change.
        present.sort_by(|a, b| match (*a == null_at, *b == null_at) {
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            _ => natural(Some(name(*a)), Some(name(*b))),
        });
        let mut names: Vec<Option<String>> = present.iter().map(|group| Some(name(*group).to_string())).collect();
        let other = names.len() as u8;
        if needs_other {
            names.push(Some(OTHER_LABEL.to_string()));
        }
        let mut lookup = vec![other; labels.len() + 1];
        for (series, group) in present.iter().enumerate() {
            lookup[*group] = series as u8;
        }
        let missing = lookup[null_at];
        Self { codes: Some(codes), lookup, missing, other, names, merged }
    }

    fn len(&self) -> usize {
        self.names.len()
    }

    #[inline]
    fn of(&self, row: usize) -> usize {
        match self.codes {
            None => 0,
            Some(codes) => match codes[row] {
                MISSING => self.missing as usize,
                OTHER => self.other as usize,
                code => self.lookup[code as usize] as usize,
            },
        }
    }
}

// ---------- bars ----------

#[derive(Clone, Copy)]
struct Tally {
    count: u64,
    sum: f64,
    min: f64,
    max: f64,
}

impl Tally {
    const EMPTY: Tally = Tally { count: 0, sum: 0.0, min: f64::INFINITY, max: f64::NEG_INFINITY };

    #[inline]
    fn add(&mut self, value: f64) {
        self.count += 1;
        self.sum += value;
        self.min = self.min.min(value);
        self.max = self.max.max(value);
    }

    fn merge(&mut self, other: &Tally) {
        self.count += other.count;
        self.sum += other.sum;
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
    }

    fn value(&self, statistic: Statistic) -> Option<f64> {
        match statistic {
            Statistic::Count => Some(self.count as f64),
            _ if self.count == 0 => None,
            Statistic::Sum => Some(self.sum),
            Statistic::Mean => Some(self.sum / self.count as f64),
            Statistic::Min => Some(self.min),
            Statistic::Max => Some(self.max),
        }
    }
}

/// The Y column as a bar reads it.
enum YValues<'a> {
    /// No Y column: every row counts once.
    None,
    Numbers(&'a [f64]),
    /// A discrete Y: a row counts if it has a value.
    Present(&'a [u32]),
}

impl YValues<'_> {
    #[inline]
    fn at(&self, row: usize) -> Option<f64> {
        match self {
            YValues::None => Some(1.0),
            YValues::Numbers(values) => Some(values[row]).filter(|v| !v.is_nan()),
            YValues::Present(codes) => (codes[row] != MISSING).then_some(1.0),
        }
    }
}

/// Bars over a discrete X (or one bar for everything when there is no X).
#[allow(clippy::too_many_arguments)]
fn by_category(
    data: &mut ChartData,
    codes: Option<&[u32]>,
    labels: &[String],
    request: &ChartRequest,
    mask: &[bool],
    y: YValues<'_>,
    statistic: Statistic,
    groups: &SeriesMap<'_>,
) {
    let series = groups.len();
    // Categories: the labels, then NULL, then the lumped rest.
    let (null_at, rest_at) = (labels.len(), labels.len() + 1);
    let mut tallies = vec![Tally::EMPTY; (labels.len() + 2) * series];
    let mut drawn = 0u64;
    for row in (0..mask.len()).filter(|row| mask[*row]) {
        let Some(value) = y.at(row) else { continue };
        let category = match codes.map(|codes| codes[row]) {
            None => 0,
            Some(MISSING) => null_at,
            Some(OTHER) => rest_at,
            Some(code) => code as usize,
        };
        tallies[category * series + groups.of(row)].add(value);
        drawn += 1;
    }
    data.rows_drawn = drawn;

    let total = |category: usize| {
        let mut all = Tally::EMPTY;
        for tally in &tallies[category * series..(category + 1) * series] {
            all.merge(tally);
        }
        all
    };
    let name = |category: usize| -> &str {
        if codes.is_none() {
            "All rows"
        } else if category == null_at {
            NULL_LABEL
        } else {
            &labels[category]
        }
    };
    let mut shown: Vec<(usize, Tally)> = (0..=null_at).map(|c| (c, total(c))).filter(|(_, t)| t.count > 0).collect();
    let limit = (request.max_categories.unwrap_or(DEFAULT_CATEGORIES as u32) as usize).clamp(1, MAX_CATEGORIES);
    let by_value = |a: &(usize, Tally), b: &(usize, Tally)| {
        let (va, vb) = (a.1.value(statistic), b.1.value(statistic));
        vb.partial_cmp(&va).unwrap_or(Ordering::Equal).then_with(|| natural(Some(name(a.0)), Some(name(b.0))))
    };
    let found = shown.len();
    let mut rest: Vec<usize> = Vec::new();
    if found > limit {
        // The largest bars are the ones kept, whatever order they are then shown in.
        shown.sort_by(by_value);
        rest = shown.split_off(limit).into_iter().map(|(category, _)| category).collect();
        data.notes.push(format!(
            "{} categories: the {} largest are shown and the rest are under {OTHER_LABEL}.",
            grouped(found as u64),
            grouped(limit as u64)
        ));
    }
    if total(rest_at).count > 0 {
        rest.push(rest_at);
        data.notes.push(format!(
            "The X column has more than {} different values; those past that number are under {OTHER_LABEL}.",
            grouped(MAX_LABELS as u64)
        ));
    }
    match request.sort {
        BarSort::Value => shown.sort_by(by_value),
        BarSort::Label => shown.sort_by(|a, b| match (a.0 == null_at, b.0 == null_at) {
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            _ => natural(Some(name(a.0)), Some(name(b.0))),
        }),
        BarSort::Seen => shown.sort_by_key(|(category, _)| *category),
    }

    let mut axis: Vec<String> = shown.iter().map(|(category, _)| name(*category).to_string()).collect();
    if !rest.is_empty() {
        axis.push(OTHER_LABEL.to_string());
    }
    data.series = (0..series)
        .map(|s| {
            let mut values: Vec<Option<f64>> =
                shown.iter().map(|(category, _)| tallies[category * series + s].value(statistic)).collect();
            if !rest.is_empty() {
                let mut merged = Tally::EMPTY;
                for category in &rest {
                    merged.merge(&tallies[category * series + s]);
                }
                values.push(merged.value(statistic));
            }
            Series { name: groups.names[s].clone(), values, points: Vec::new() }
        })
        .collect();
    data.x = XAxis::Categories { labels: axis };
}

enum Binning {
    /// `count` bins of `step` from `start`.
    Width { start: f64, step: f64, count: usize },
    /// `count` time buckets from bucket number `first`.
    Calendar { unit: TimeBucket, first: i64, count: usize },
}

impl Binning {
    fn count(&self) -> usize {
        match self {
            Binning::Width { count, .. } | Binning::Calendar { count, .. } => *count,
        }
    }

    #[inline]
    fn index(&self, value: f64) -> usize {
        match self {
            Binning::Width { start, step, count } => (((value - start) / step).floor().max(0.0) as usize).min(count - 1),
            Binning::Calendar { unit, first, count } => ((unit.index(value) - first).max(0) as usize).min(count - 1),
        }
    }

    fn edges(&self) -> Vec<f64> {
        match self {
            Binning::Width { start, step, count } => (0..=*count).map(|i| start + i as f64 * step).collect(),
            Binning::Calendar { unit, first, count } => (0..=*count as i64).map(|i| unit.start(first + i)).collect(),
        }
    }
}

/// A step of 1, 2, 2.5 or 5 times a power of ten, at least `raw`, so bin edges are round numbers.
fn nice_step(raw: f64) -> f64 {
    let power = 10f64.powf(raw.log10().floor());
    let fraction = raw / power;
    let nice = [1.0, 2.0, 2.5, 5.0].into_iter().find(|n| fraction <= *n + 1e-9).unwrap_or(10.0);
    nice * power
}

fn binning(lo: f64, hi: f64, time: bool, request: &ChartRequest, drawn: u64, notes: &mut Vec<String>) -> Binning {
    if time {
        let buckets = |unit: TimeBucket| unit.index(hi) - unit.index(lo) + 1;
        let mut unit = match request.bucket {
            Some(unit) => unit,
            // The finest unit that still gives a readable number of bars.
            None => TimeBucket::ALL.into_iter().find(|unit| buckets(*unit) <= 60).unwrap_or(TimeBucket::Year),
        };
        let asked = unit;
        while unit != TimeBucket::Year && buckets(unit) > MAX_BUCKETS {
            unit = TimeBucket::ALL[TimeBucket::ALL.iter().position(|u| *u == unit).expect("listed") + 1];
        }
        if unit != asked {
            notes.push(format!(
                "By {} there would be more than {} bars, so the chart is by {}.",
                format!("{asked:?}").to_lowercase(),
                grouped(MAX_BUCKETS as u64),
                format!("{unit:?}").to_lowercase()
            ));
        }
        return Binning::Calendar { unit, first: unit.index(lo), count: buckets(unit) as usize };
    }
    let target = match request.bins {
        Some(bins) => (bins as usize).clamp(1, MAX_BINS),
        None => ((((drawn.max(1) as f64).log2().ceil() + 1.0) * 2.0) as usize).clamp(10, 60),
    };
    if hi <= lo {
        // Every value is the same: one bin around it.
        let step = if lo == 0.0 { 1.0 } else { nice_step(lo.abs() / 10.0) };
        return Binning::Width { start: (lo / step).floor() * step, step, count: 1 };
    }
    let step = nice_step((hi - lo) / target as f64);
    let start = (lo / step).floor() * step;
    let count = (((hi - start) / step).floor() as usize + 1).min(MAX_BINS * 2);
    Binning::Width { start, step, count }
}

/// Bars over a continuous or time X, each covering a range of it.
#[allow(clippy::too_many_arguments)]
fn binned(
    data: &mut ChartData,
    x: &[f64],
    time: bool,
    request: &ChartRequest,
    mask: &[bool],
    y: YValues<'_>,
    statistic: Statistic,
    groups: &SeriesMap<'_>,
) {
    let usable = |row: &usize| mask[*row] && !x[*row].is_nan();
    let (mut lo, mut hi, mut drawn) = (f64::INFINITY, f64::NEG_INFINITY, 0u64);
    for row in (0..mask.len()).filter(usable) {
        if y.at(row).is_some() {
            lo = lo.min(x[row]);
            hi = hi.max(x[row]);
            drawn += 1;
        }
    }
    data.rows_drawn = drawn;
    if drawn == 0 {
        data.x = XAxis::Bins { edges: Vec::new(), time, bucket: None };
        return;
    }
    let bins = binning(lo, hi, time, request, drawn, &mut data.notes);
    let series = groups.len();
    let mut tallies = vec![Tally::EMPTY; bins.count() * series];
    for row in (0..mask.len()).filter(usable) {
        if let Some(value) = y.at(row) {
            tallies[bins.index(x[row]) * series + groups.of(row)].add(value);
        }
    }
    data.series = (0..series)
        .map(|s| Series {
            name: groups.names[s].clone(),
            values: (0..bins.count()).map(|bin| tallies[bin * series + s].value(statistic)).collect(),
            points: Vec::new(),
        })
        .collect();
    data.x = XAxis::Bins {
        edges: bins.edges(),
        time,
        bucket: match bins {
            Binning::Calendar { unit, .. } => Some(unit),
            Binning::Width { .. } => None,
        },
    };
}

// ---------- scatter ----------

fn scatter(data: &mut ChartData, x: &[f64], y: &[f64], mask: &[bool], groups: &SeriesMap<'_>, large: Large) {
    let usable = |row: &usize| mask[*row] && !x[*row].is_nan() && !y[*row].is_nan();
    let (mut x_lo, mut x_hi, mut y_lo, mut y_hi, mut drawn) =
        (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY, 0usize);
    for row in (0..mask.len()).filter(usable) {
        x_lo = x_lo.min(x[row]);
        x_hi = x_hi.max(x[row]);
        y_lo = y_lo.min(y[row]);
        y_hi = y_hi.max(y[row]);
        drawn += 1;
    }
    data.rows_drawn = drawn as u64;
    if drawn == 0 {
        return;
    }
    if drawn > MAX_POINTS && large == Large::Density {
        let (x_span, y_span) = ((x_hi - x_lo).max(f64::MIN_POSITIVE), (y_hi - y_lo).max(f64::MIN_POSITIVE));
        let mut counts = vec![0u32; DENSITY_COLS * DENSITY_ROWS];
        for row in (0..mask.len()).filter(usable) {
            let col = (((x[row] - x_lo) / x_span * DENSITY_COLS as f64) as usize).min(DENSITY_COLS - 1);
            let line = (((y[row] - y_lo) / y_span * DENSITY_ROWS as f64) as usize).min(DENSITY_ROWS - 1);
            counts[line * DENSITY_COLS + col] += 1;
        }
        data.density = Some(Density {
            cols: DENSITY_COLS,
            rows: DENSITY_ROWS,
            x: [x_lo, x_hi],
            y: [y_lo, y_hi],
            max: counts.iter().copied().max().unwrap_or(0),
            counts,
        });
        data.notes.push(format!(
            "{} points are too many to draw one by one. The shading shows how many fall in each cell; zoom in, or filter, to see the points.",
            grouped(drawn as u64)
        ));
        return;
    }
    // Past the limit, keep each point with the same chance. The choice depends
    // only on the row number, so the same rows are picked every time.
    let threshold = if drawn > MAX_POINTS { (MAX_POINTS as f64 / drawn as f64 * u32::MAX as f64) as u32 } else { u32::MAX };
    data.sampled = drawn > MAX_POINTS;
    let mut series: Vec<Series> =
        groups.names.iter().map(|name| Series { name: name.clone(), values: Vec::new(), points: Vec::new() }).collect();
    for row in (0..mask.len()).filter(usable) {
        if (row as u32).wrapping_mul(2_654_435_761) <= threshold {
            series[groups.of(row)].points.extend([x[row], y[row]]);
        }
    }
    if data.sampled {
        let shown: usize = series.iter().map(|s| s.points.len() / 2).sum();
        data.notes.push(format!(
            "A random sample of {} of {} points is drawn.",
            grouped(shown as u64),
            grouped(drawn as u64)
        ));
    }
    data.series = series;
}

// ---------- small helpers ----------

/// Orders names the way a person would: numbers by value, text ignoring case, NULL last.
fn natural(a: Option<&str>, b: Option<&str>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, _) => Ordering::Greater,
        (_, None) => Ordering::Less,
        (Some(a), Some(b)) => match (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
            (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            _ => a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b)),
        },
    }
}

/// A count with thousands separators.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ColumnInfo, ColumnKind, RowBatch};

    /// region, amount, placed (date and time), status
    fn orders() -> Dataset {
        let rows: &[[Option<&str>; 4]] = &[
            [Some("north"), Some("10"), Some("2026-01-05 09:00:00"), Some("paid")],
            [Some("north"), Some("30"), Some("2026-01-20 10:30:00"), Some("paid")],
            [Some("south"), Some("5"), Some("2026-02-01 00:00:00"), Some("open")],
            [Some("south"), Some("15.5"), Some("2026-02-14 23:59:59"), Some("paid")],
            [Some("east"), Some("100"), Some("2026-04-02 12:00:00"), None],
            [None, Some("7"), Some("0000-00-00 00:00:00"), Some("open")],
            [Some("east"), None, Some("2026-04-30 08:00:00"), Some("paid")],
        ];
        dataset(&["region", "amount", "placed", "status"], rows.iter().map(|r| r.to_vec()))
    }

    fn dataset<'a>(names: &[&str], rows: impl IntoIterator<Item = Vec<Option<&'a str>>>) -> Dataset {
        let store = ResultStore::new();
        store.set_columns(names.iter().map(|n| ColumnInfo::plain(n, ColumnKind::Text)).collect());
        let mut batch = RowBatch::default();
        for row in rows {
            batch.push_row(row.iter().map(|c| c.map(str::as_bytes)));
        }
        store.append(&mut batch);
        store.finish(None);
        Dataset::new(Arc::new(store))
    }

    fn request(kind: ChartKind) -> ChartRequest {
        ChartRequest::new(kind)
    }

    fn axis(column: usize, role: Role) -> Option<AxisRef> {
        Some(AxisRef { column, role })
    }

    fn labels(data: &ChartData) -> Vec<&str> {
        match &data.x {
            XAxis::Categories { labels } => labels.iter().map(String::as_str).collect(),
            other => panic!("not categories: {other:?}"),
        }
    }

    fn edges(data: &ChartData) -> &[f64] {
        match &data.x {
            XAxis::Bins { edges, .. } => edges,
            other => panic!("not bins: {other:?}"),
        }
    }

    fn date(text: &str) -> f64 {
        parse_time(text.as_bytes()).expect("a date").0
    }

    #[test]
    fn dates_and_times_are_read_as_mysql_writes_them() {
        assert_eq!(parse_time(b"1970-01-01"), Some((0.0, true)));
        assert_eq!(parse_time(b"1970-01-02 00:00:01"), Some((DAY_MS + 1_000.0, true)));
        assert_eq!(parse_time(b"2026-11-16 12:34:56.250000"), Some((date("2026-11-16") + 45_296_250.0, true)));
        assert_eq!(date("2000-03-01") - date("2000-02-28"), 2.0 * DAY_MS, "2000 was a leap year");
        assert_eq!(date("1900-03-01") - date("1900-02-28"), DAY_MS, "1900 was not");
        assert_eq!(date("1969-12-31"), -DAY_MS);
        // A TIME: no date, may be negative or longer than a day.
        assert_eq!(parse_time(b"01:30:00"), Some((5_400_000.0, false)));
        assert_eq!(parse_time(b"-838:59:59"), Some((-(838.0 * 3600.0 + 59.0 * 60.0 + 59.0) * 1000.0, false)));
        for not_a_time in ["0000-00-00", "0000-00-00 00:00:00", "2026-00-10", "2026-13-01", "12", "abc", "", "2026-01-01x", "10:99:00"] {
            assert_eq!(parse_time(not_a_time.as_bytes()), None, "{not_a_time:?}");
        }
    }

    #[test]
    fn calendar_arithmetic_round_trips() {
        for days in [-800_000, -1, 0, 1, 59, 60, 365, 11_016, 20_000, 2_932_896] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days, "{y}-{m}-{d}");
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
    }

    #[test]
    fn time_buckets_start_where_the_calendar_says() {
        let at = date("2026-11-18") + 13.0 * 3_600_000.0; // a Wednesday afternoon
        for (unit, start) in [
            (TimeBucket::Hour, "2026-11-18 13:00:00"),
            (TimeBucket::Day, "2026-11-18"),
            (TimeBucket::Week, "2026-11-16"),
            (TimeBucket::Month, "2026-11-01"),
            (TimeBucket::Quarter, "2026-10-01"),
            (TimeBucket::Year, "2026-01-01"),
        ] {
            assert_eq!(unit.start(unit.index(at)), date(start), "{unit:?}");
        }
        // Before 1970 too.
        assert_eq!(TimeBucket::Month.start(TimeBucket::Month.index(date("1969-12-31"))), date("1969-12-01"));
        assert_eq!(TimeBucket::Week.start(TimeBucket::Week.index(date("1969-12-31"))), date("1969-12-29"));
    }

    #[test]
    fn bars_count_rows_per_category_largest_first() {
        let data = orders().chart(&ChartRequest { x: axis(0, Role::Discrete), ..request(ChartKind::Bar) }).unwrap();
        // Two each for north, south and east: ties go in name order. NULL is a category of its own.
        assert_eq!(labels(&data), ["east", "north", "south", NULL_LABEL]);
        assert_eq!(data.series.len(), 1);
        assert_eq!(data.series[0].name, None);
        assert_eq!(data.series[0].values, [Some(2.0), Some(2.0), Some(2.0), Some(1.0)]);
        assert_eq!((data.rows_total, data.rows_filtered, data.rows_drawn), (7, 7, 7));
    }

    #[test]
    fn bars_show_a_statistic_of_y_and_skip_rows_without_one() {
        let chart = |statistic| {
            let data = orders()
                .chart(&ChartRequest {
                    x: axis(0, Role::Discrete),
                    y: axis(1, Role::Continuous),
                    statistic,
                    sort: BarSort::Label,
                    ..request(ChartKind::Bar)
                })
                .unwrap();
            assert_eq!(labels(&data), ["east", "north", "south", NULL_LABEL]);
            assert_eq!(data.rows_drawn, 6, "one row has no amount");
            data.series[0].values.clone()
        };
        assert_eq!(chart(Statistic::Sum), [Some(100.0), Some(40.0), Some(20.5), Some(7.0)]);
        assert_eq!(chart(Statistic::Mean), [Some(100.0), Some(20.0), Some(10.25), Some(7.0)]);
        assert_eq!(chart(Statistic::Min), [Some(100.0), Some(10.0), Some(5.0), Some(7.0)]);
        assert_eq!(chart(Statistic::Max), [Some(100.0), Some(30.0), Some(15.5), Some(7.0)]);
        assert_eq!(chart(Statistic::Count), [Some(1.0), Some(2.0), Some(2.0), Some(1.0)]);
    }

    #[test]
    fn a_colour_column_splits_bars_into_named_series() {
        let data = orders()
            .chart(&ChartRequest {
                x: axis(0, Role::Discrete),
                y: axis(1, Role::Continuous),
                color: Some(3),
                statistic: Statistic::Sum,
                sort: BarSort::Label,
                ..request(ChartKind::Bar)
            })
            .unwrap();
        assert_eq!(labels(&data), ["east", "north", "south", NULL_LABEL]);
        let series: Vec<(Option<&str>, &[Option<f64>])> =
            data.series.iter().map(|s| (s.name.as_deref(), s.values.as_slice())).collect();
        assert_eq!(
            series,
            [
                (Some("open"), &[None, None, Some(5.0), Some(7.0)][..]),
                (Some("paid"), &[None, Some(40.0), Some(15.5), None][..]),
                (Some(NULL_LABEL), &[Some(100.0), None, None, None][..]),
            ]
        );
    }

    #[test]
    fn categories_past_the_limit_are_merged_into_other() {
        let rows: Vec<Vec<Option<String>>> =
            (0..10).flat_map(|c| (0..=c).map(move |_| vec![Some(format!("c{c}")), Some("2".to_string())])).collect();
        let data = dataset(&["c", "n"], rows.iter().map(|r| r.iter().map(|c| c.as_deref()).collect()));
        let chart = data
            .chart(&ChartRequest {
                x: axis(0, Role::Discrete),
                y: axis(1, Role::Continuous),
                statistic: Statistic::Sum,
                max_categories: Some(3),
                ..request(ChartKind::Bar)
            })
            .unwrap();
        // c9 has ten rows of 2, c8 nine, c7 eight; the other seven have 1 + 2 + … + 7 = 28 rows.
        assert_eq!(labels(&chart), ["c9", "c8", "c7", OTHER_LABEL]);
        assert_eq!(chart.series[0].values, [Some(20.0), Some(18.0), Some(16.0), Some(56.0)]);
        assert!(chart.notes[0].contains("10 categories"), "{:?}", chart.notes);
        // Sorted by name, the same three are kept.
        let by_name = data
            .chart(&ChartRequest { x: axis(0, Role::Discrete), max_categories: Some(3), sort: BarSort::Label, ..request(ChartKind::Bar) })
            .unwrap();
        assert_eq!(labels(&by_name), ["c7", "c8", "c9", OTHER_LABEL]);
        assert_eq!(by_name.series[0].values, [Some(8.0), Some(9.0), Some(10.0), Some(28.0)]);
    }

    #[test]
    fn names_that_are_numbers_sort_by_value() {
        let data = dataset(&["n"], ["10", "9", "100", "9"].map(|v| vec![Some(v)]));
        let chart = data.chart(&ChartRequest { x: axis(0, Role::Discrete), sort: BarSort::Label, ..request(ChartKind::Bar) }).unwrap();
        assert_eq!(labels(&chart), ["9", "10", "100"]);
        let seen = data.chart(&ChartRequest { x: axis(0, Role::Discrete), sort: BarSort::Seen, ..request(ChartKind::Bar) }).unwrap();
        assert_eq!(labels(&seen), ["10", "9", "100"]);
    }

    #[test]
    fn a_histogram_counts_values_in_round_bins() {
        let values: Vec<String> = (0..100).map(|n| n.to_string()).collect();
        let data = dataset(&["n"], values.iter().map(|v| vec![Some(v.as_str())]));
        let chart = data.chart(&ChartRequest { x: axis(0, Role::Continuous), bins: Some(10), ..request(ChartKind::Histogram) }).unwrap();
        assert_eq!(edges(&chart), [0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0]);
        assert_eq!(chart.series[0].values, vec![Some(10.0); 10]);
        assert_eq!(chart.x_extent, Some([0.0, 99.0]));
        // The largest value sitting exactly on an edge still has a bin.
        let to_100: Vec<String> = (0..=100).map(|n| n.to_string()).collect();
        let chart = dataset(&["n"], to_100.iter().map(|v| vec![Some(v.as_str())]))
            .chart(&ChartRequest { x: axis(0, Role::Continuous), bins: Some(10), ..request(ChartKind::Histogram) })
            .unwrap();
        assert_eq!(chart.series[0].values.len(), 11);
        assert_eq!(chart.series[0].values[10], Some(1.0));
        assert_eq!(chart.series[0].values.iter().flatten().sum::<f64>(), 101.0);
    }

    #[test]
    fn a_histogram_of_one_value_or_no_values_is_still_a_chart() {
        let same = dataset(&["n"], ["7", "7", "7"].map(|v| vec![Some(v)]));
        let chart = same.chart(&ChartRequest { x: axis(0, Role::Continuous), ..request(ChartKind::Histogram) }).unwrap();
        assert_eq!(chart.series[0].values, [Some(3.0)]);
        let e = edges(&chart);
        assert!(e[0] <= 7.0 && 7.0 < e[1], "{e:?}");

        let text = dataset(&["n"], ["a", "b"].map(|v| vec![Some(v)]));
        let chart = text.chart(&ChartRequest { x: axis(0, Role::Continuous), ..request(ChartKind::Histogram) }).unwrap();
        assert_eq!((chart.rows_filtered, chart.rows_drawn), (2, 0));
        assert!(chart.series.is_empty());
        assert!(text.chart(&ChartRequest { x: axis(0, Role::Discrete), ..request(ChartKind::Histogram) }).is_err());
    }

    #[test]
    fn time_on_x_is_cut_into_calendar_buckets_including_empty_ones() {
        let data = orders()
            .chart(&ChartRequest {
                x: axis(2, Role::Time),
                y: axis(1, Role::Continuous),
                statistic: Statistic::Sum,
                bucket: Some(TimeBucket::Month),
                ..request(ChartKind::Bar)
            })
            .unwrap();
        // January to April; nothing in March; the zero date and the row without an amount are left out.
        assert_eq!(edges(&data), ["2026-01-01", "2026-02-01", "2026-03-01", "2026-04-01", "2026-05-01"].map(date));
        assert_eq!(data.series[0].values, [Some(40.0), Some(20.5), None, Some(100.0)]);
        assert_eq!(data.rows_drawn, 5);
        assert!(matches!(data.x, XAxis::Bins { time: true, bucket: Some(TimeBucket::Month), .. }));
        assert!(!data.x_time_only);
    }

    #[test]
    fn the_time_unit_is_chosen_to_give_a_readable_number_of_bars() {
        let chart = |bucket| {
            let data = orders().chart(&ChartRequest { x: axis(2, Role::Time), bucket, ..request(ChartKind::Histogram) }).unwrap();
            match data.x {
                XAxis::Bins { bucket, edges, .. } => (bucket.unwrap(), edges.len() - 1, data.notes),
                other => panic!("{other:?}"),
            }
        };
        // 5 January to 30 April is 116 days: too many bars by day, 17 by week.
        let (unit, bars, _) = chart(None);
        assert_eq!((unit, bars), (TimeBucket::Week, 17));
        // By second it would be ten million bars; the unit is made coarser and the chart says so.
        let (unit, bars, notes) = chart(Some(TimeBucket::Second));
        assert_eq!(unit, TimeBucket::Day);
        assert_eq!(bars, 116);
        assert!(notes[0].contains("by day"), "{notes:?}");
    }

    #[test]
    fn filters_keep_rows_by_range_by_value_and_by_null() {
        let chart = |filters: Vec<Filter>| {
            let data = orders()
                .chart(&ChartRequest { x: axis(0, Role::Discrete), sort: BarSort::Label, filters, ..request(ChartKind::Bar) })
                .unwrap();
            (labels(&data).join(","), data.series[0].values.iter().map(|v| v.unwrap() as u32).collect::<Vec<_>>(), data.rows_filtered)
        };
        let filter = |column, role| Filter { column, role, min: None, max: None, values: Vec::new() };
        // Amount from 10 to 100: the 5, the 7 and the row with no amount go.
        let range = Filter { min: Some(10.0), max: Some(100.0), ..filter(1, Role::Continuous) };
        assert_eq!(chart(vec![range.clone()]), ("east,north,south".to_string(), vec![1, 2, 1], 4));
        // Only a lower bound.
        assert_eq!(chart(vec![Filter { min: Some(16.0), ..filter(1, Role::Continuous) }]).2, 2);
        // Status paid, or NULL.
        let paid = Filter { values: vec![Some("paid".into())], ..filter(3, Role::Discrete) };
        assert_eq!(chart(vec![paid.clone()]), ("east,north,south".to_string(), vec![1, 2, 1], 4));
        let none = Filter { values: vec![None], ..filter(3, Role::Discrete) };
        assert_eq!(chart(vec![none]), ("east".to_string(), vec![1], 1));
        // Both at once narrow further.
        assert_eq!(chart(vec![range, paid]), ("north,south".to_string(), vec![2, 1], 3));
        // A filter with nothing chosen keeps everything.
        assert_eq!(chart(vec![filter(3, Role::Discrete), filter(1, Role::Continuous)]).2, 7);
        // A time range.
        let february = Filter { min: Some(date("2026-02-01")), max: Some(date("2026-02-28 23:59:59")), ..filter(2, Role::Time) };
        assert_eq!(chart(vec![february]), ("south".to_string(), vec![2], 2));
    }

    #[test]
    fn the_x_range_narrows_the_chart_but_not_the_extent_it_reports() {
        let data = orders()
            .chart(&ChartRequest {
                x: axis(2, Role::Time),
                bucket: Some(TimeBucket::Month),
                x_range: Some([date("2026-02-01"), date("2026-03-01")]),
                ..request(ChartKind::Histogram)
            })
            .unwrap();
        assert_eq!(edges(&data), ["2026-02-01", "2026-03-01"].map(date));
        assert_eq!(data.series[0].values, [Some(2.0)]);
        assert_eq!(data.x_extent, Some([date("2026-01-05 09:00:00"), date("2026-04-30 08:00:00")]));
    }

    #[test]
    fn a_scatter_plot_sends_points_grouped_by_colour() {
        let data = orders()
            .chart(&ChartRequest { x: axis(2, Role::Time), y: axis(1, Role::Continuous), color: Some(0), ..request(ChartKind::Scatter) })
            .unwrap();
        assert_eq!(data.x, XAxis::Continuous { time: true });
        // Five rows have both a date and an amount.
        assert_eq!(data.rows_drawn, 5);
        let series: Vec<(&str, &[f64])> = data.series.iter().map(|s| (s.name.as_deref().unwrap(), s.points.as_slice())).collect();
        assert_eq!(series[0], ("east", &[date("2026-04-02 12:00:00"), 100.0][..]));
        assert_eq!(series[1].0, "north");
        assert_eq!(series[1].1.len(), 4);
        assert_eq!(series[2].0, "south");
        assert!(!data.sampled && data.density.is_none());
        assert!(orders().chart(&ChartRequest { x: axis(0, Role::Discrete), y: axis(1, Role::Continuous), ..request(ChartKind::Scatter) }).is_err());
    }

    fn many_points(count: usize) -> Dataset {
        let rows: Vec<(String, String)> = (0..count).map(|i| ((i % 1000).to_string(), (i / 1000).to_string())).collect();
        dataset(&["x", "y"], rows.iter().map(|(x, y)| vec![Some(x.as_str()), Some(y.as_str())]))
    }

    #[test]
    fn too_many_points_become_a_grid_of_counts() {
        let count = MAX_POINTS + 10_000;
        let data = many_points(count)
            .chart(&ChartRequest { x: axis(0, Role::Continuous), y: axis(1, Role::Continuous), ..request(ChartKind::Scatter) })
            .unwrap();
        let density = data.density.expect("a density grid");
        assert!(data.series.is_empty());
        assert_eq!(density.counts.iter().map(|c| *c as usize).sum::<usize>(), count, "every point is counted once");
        assert_eq!((density.x, density.y), ([0.0, 999.0], [0.0, 59.0]));
        assert_eq!(density.max, *density.counts.iter().max().unwrap());
        assert_eq!(data.rows_drawn as usize, count);
    }

    #[test]
    fn or_a_sample_of_about_the_limit_that_is_the_same_each_time() {
        let count = MAX_POINTS * 4;
        let data = many_points(count);
        let chart = || {
            data.chart(&ChartRequest {
                x: axis(0, Role::Continuous),
                y: axis(1, Role::Continuous),
                large: Large::Sample,
                ..request(ChartKind::Scatter)
            })
            .unwrap()
        };
        let first = chart();
        let shown = first.series[0].points.len() / 2;
        assert!(first.sampled);
        assert!((MAX_POINTS * 9 / 10..=MAX_POINTS * 11 / 10).contains(&shown), "{shown} points");
        assert_eq!(first.rows_drawn as usize, count);
        assert_eq!(chart(), first);
        // Zoomed in to fewer points than the limit, they are all sent.
        let zoomed = data
            .chart(&ChartRequest {
                x: axis(0, Role::Continuous),
                y: axis(1, Role::Continuous),
                x_range: Some([0.0, 99.0]),
                y_range: Some([0.0, 49.0]),
                ..request(ChartKind::Scatter)
            })
            .unwrap();
        assert!(!zoomed.sampled && zoomed.density.is_none());
        assert_eq!(zoomed.series[0].points.len() / 2, 100 * 50);
    }

    #[test]
    fn more_colour_groups_than_fit_are_merged_keeping_the_largest() {
        // Group g0 has one row, g1 two, and so on up to g19.
        let rows: Vec<String> = (0..20).flat_map(|g| (0..=g).map(move |_| format!("g{g}"))).collect();
        let data = dataset(&["g"], rows.iter().map(|g| vec![Some(g.as_str())]));
        let chart = data.chart(&ChartRequest { color: Some(0), ..request(ChartKind::Bar) }).unwrap();
        assert_eq!(labels(&chart), ["All rows"]);
        let names: Vec<&str> = chart.series.iter().map(|s| s.name.as_deref().unwrap()).collect();
        assert_eq!(names.len(), MAX_SERIES);
        assert_eq!(names[MAX_SERIES - 1], OTHER_LABEL);
        // The seven largest are g13 to g19; g0 to g12 hold 1 + 2 + … + 13 = 91 rows.
        assert!(names.contains(&"g19") && names.contains(&"g13") && !names.contains(&"g12"), "{names:?}");
        assert_eq!(chart.series[MAX_SERIES - 1].values, [Some(91.0)]);
        assert_eq!(chart.series.iter().map(|s| s.values[0].unwrap()).sum::<f64>(), 210.0);
        assert!(chart.notes[0].contains("colour column"), "{:?}", chart.notes);
    }

    #[test]
    fn a_column_with_too_many_values_lumps_the_rest_together() {
        let values: Vec<String> = (0..MAX_LABELS + 50).map(|i| format!("v{i}")).collect();
        let data = dataset(&["v"], values.iter().map(|v| vec![Some(v.as_str())]));
        let summary = data.summary(0, Role::Discrete).unwrap();
        assert_eq!((summary.distinct as usize, summary.overflow), (MAX_LABELS, true));
        assert_eq!(summary.categories.len(), MAX_LISTED);
        let chart = data.chart(&ChartRequest { x: axis(0, Role::Discrete), max_categories: Some(5), ..request(ChartKind::Bar) }).unwrap();
        assert_eq!(labels(&chart).last(), Some(&OTHER_LABEL));
        assert_eq!(chart.series[0].values.iter().flatten().sum::<f64>() as usize, MAX_LABELS + 50, "no row is lost");
    }

    #[test]
    fn a_summary_says_what_a_column_holds_in_each_role() {
        let data = orders();
        let amount = data.summary(1, Role::Continuous).unwrap();
        assert_eq!((amount.rows, amount.missing, amount.min, amount.max), (7, 1, Some(5.0), Some(100.0)));
        let placed = data.summary(2, Role::Time).unwrap();
        assert_eq!((placed.missing, placed.time_only), (1, false), "the zero date is missing");
        assert_eq!(placed.min, Some(date("2026-01-05 09:00:00")));
        let status = data.summary(3, Role::Discrete).unwrap();
        assert_eq!(status.distinct, 2);
        assert_eq!(
            status.categories,
            [
                CategoryCount { label: Some("paid".into()), rows: 4 },
                CategoryCount { label: Some("open".into()), rows: 2 },
                CategoryCount { label: None, rows: 1 },
            ]
        );
        // Text read as numbers: nothing usable, and it says so rather than failing.
        let region = data.summary(0, Role::Continuous).unwrap();
        assert_eq!((region.missing, region.min), (7, None));
        assert!(data.summary(9, Role::Discrete).is_err());
    }

    #[test]
    fn a_time_of_day_column_is_marked_as_such() {
        let data = dataset(&["t"], ["08:00:00", "17:30:00"].map(|v| vec![Some(v)]));
        let summary = data.summary(0, Role::Time).unwrap();
        assert!(summary.time_only);
        assert_eq!((summary.min, summary.max), (Some(28_800_000.0), Some(63_000_000.0)));
        let chart = data.chart(&ChartRequest { x: axis(0, Role::Time), ..request(ChartKind::Histogram) }).unwrap();
        assert!(chart.x_time_only);
    }

    #[test]
    fn long_values_are_shortened_in_names() {
        let long = "x".repeat(400);
        let data = dataset(&["v"], [vec![Some(long.as_str())]]);
        let summary = data.summary(0, Role::Discrete).unwrap();
        let label = summary.categories[0].label.clone().unwrap();
        assert_eq!(label.chars().count(), LABEL_CHARS + 1);
        assert!(label.ends_with('…'));
    }

    #[test]
    fn columns_are_read_once_and_their_memory_is_given_back() {
        let data = many_points(10_000);
        assert_eq!(data.kept_bytes(), 0);
        data.summary(0, Role::Continuous).unwrap();
        let one = data.kept_bytes();
        assert_eq!(one, 80_000);
        data.summary(0, Role::Continuous).unwrap();
        assert_eq!(data.kept_bytes(), one, "asking again reads nothing new");
        data.chart(&ChartRequest { x: axis(0, Role::Continuous), y: axis(1, Role::Continuous), ..request(ChartKind::Scatter) }).unwrap();
        assert_eq!(data.kept_bytes(), 2 * one);
    }

    #[test]
    fn columns_no_longer_on_the_chart_are_let_go_when_many_are_kept() {
        let names: Vec<String> = (0..12).map(|c| format!("c{c}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let data = dataset(&names, (0..100).map(|_| vec![Some("1"); 12]));
        for column in 0..12 {
            data.summary(column, Role::Continuous).unwrap();
            assert!(data.kept_bytes() <= (MAX_KEPT_COLUMNS * 800) as u64, "after column {column}");
        }
        // The ninth column cleared out the eight before it; three more have been read since.
        assert_eq!(data.kept_bytes(), 4 * 800);
    }

    #[test]
    fn a_result_that_is_still_loading_cannot_be_charted() {
        let store = ResultStore::new();
        store.set_columns(vec![ColumnInfo::plain("n", ColumnKind::Number)]);
        let data = Dataset::new(Arc::new(store));
        assert!(data.summary(0, Role::Continuous).unwrap_err().to_string().contains("still loading"));
    }

    #[test]
    fn grouped_counts_read_easily() {
        assert_eq!([0, 999, 1_000, 6_000_000].map(grouped), ["0", "999", "1,000", "6,000,000"]);
    }
}
