//! How long charting a very large result takes.
//!
//! Ignored by default: it builds six million rows. Run it optimised, or the
//! times mean nothing:
//!
//! ```text
//! cargo test -p sqltool-core --release --test chart_speed -- --ignored --nocapture
//! ```
//!
//! It also checks the answers, so it doubles as a test that nothing is lost
//! or counted twice at that size, including for rows that went to disk.

use std::sync::Arc;
use std::time::Instant;

use sqltool_core::chart::{AxisRef, ChartKind, ChartRequest, Dataset, Filter, Large, Role, Statistic, TimeBucket, TrendKind, TrendSpec, XAxis};
use sqltool_core::store::memory_in_use;
use sqltool_core::{ColumnInfo, ColumnKind, ResultStore, RowBatch};

const ROWS: usize = 6_000_000;
const REGIONS: [&str; 5] = ["north", "south", "east", "west", "central"];

/// id, region, amount, placed, note. Ten rows a minute from 2026-01-01.
fn orders() -> Arc<ResultStore> {
    // 64 MB in memory and the rest on disk, as a result this size would be in the app.
    let store = ResultStore::with_limits(64 * 1024 * 1024, 16 * 1024 * 1024);
    store.set_columns(vec![
        ColumnInfo::plain("id", ColumnKind::Number),
        ColumnInfo::plain("region", ColumnKind::Text),
        ColumnInfo::plain("amount", ColumnKind::Number),
        ColumnInfo::plain("placed", ColumnKind::Date),
        ColumnInfo::plain("note", ColumnKind::Text),
    ]);
    let mut batch = RowBatch::default();
    for row in 0..ROWS {
        let minutes = row / 10;
        let (day, hour, minute) = (minutes / 1440, minutes / 60 % 24, minutes % 60);
        // 2026 has 365 days; the rows run a little over a year.
        let (year, day_of_year) = (2026 + day / 365, day % 365);
        let (month, day_of_month) = month_and_day(day_of_year);
        let cells = [
            row.to_string(),
            REGIONS[row % REGIONS.len()].to_string(),
            format!("{}.{:02}", row % 1000, row % 100),
            format!("{year}-{month:02}-{day_of_month:02} {hour:02}:{minute:02}:00"),
            "a note of ordinary length, as most rows have".to_string(),
        ];
        batch.push_row(cells.iter().map(|c| Some(c.as_bytes())));
        if batch.bytes() > 4 * 1024 * 1024 {
            store.append(&mut batch);
        }
    }
    store.append(&mut batch);
    store.finish(None);
    Arc::new(store)
}

fn month_and_day(day_of_year: usize) -> (usize, usize) {
    let mut left = day_of_year;
    for (month, days) in [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31].into_iter().enumerate() {
        if left < days {
            return (month + 1, left + 1);
        }
        left -= days;
    }
    unreachable!("a day of a 365-day year")
}

fn timed<T>(what: &str, work: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let out = work();
    println!("{:>8.0} ms  {what}", start.elapsed().as_secs_f64() * 1000.0);
    out
}

fn axis(column: usize, role: Role) -> Option<AxisRef> {
    Some(AxisRef { column, role })
}

#[test]
#[ignore = "builds six million rows; run with --release"]
fn charting_six_million_rows() {
    let store = timed("build the result", orders);
    let status = store.status();
    println!("{} rows, {} MB, {} MB of it on disk", status.total_rows, status.bytes >> 20, status.spilled_bytes >> 20);
    assert!(status.spilled_bytes > 0, "the test is meant to cover rows on disk");
    let before = memory_in_use();
    let data = Dataset::new(store);

    // Bars: amount summed by region. Reads two columns.
    let bars = ChartRequest { x: axis(1, Role::Discrete), y: axis(2, Role::Continuous), statistic: Statistic::Sum, ..ChartRequest::new(ChartKind::Bar) };
    let chart = timed("bar, first time (reads region and amount)", || data.chart(&bars).unwrap());
    assert_eq!(chart.rows_drawn as usize, ROWS);
    assert_eq!(chart.series[0].values.len(), REGIONS.len());
    let total: f64 = chart.series[0].values.iter().flatten().sum();
    let expected: f64 = (0..ROWS).map(|row| (row % 1000) as f64 + (row % 100) as f64 / 100.0).sum();
    assert!((total - expected).abs() < 1.0, "{total} against {expected}");
    timed("bar, again", || data.chart(&bars).unwrap());

    // With a filter being dragged: every change is a new request over kept columns.
    let filtered = ChartRequest {
        filters: vec![Filter { column: 2, role: Role::Continuous, min: Some(100.0), max: Some(600.0), values: Vec::new() }],
        color: Some(1),
        ..bars.clone()
    };
    let chart = timed("bar with a range filter and colour", || data.chart(&filtered).unwrap());
    assert!(chart.rows_filtered > 0 && (chart.rows_filtered as usize) < ROWS);

    // Histogram of amount.
    let histogram = ChartRequest { x: axis(2, Role::Continuous), ..ChartRequest::new(ChartKind::Histogram) };
    let chart = timed("histogram of amount", || data.chart(&histogram).unwrap());
    assert_eq!(chart.series[0].values.iter().flatten().sum::<f64>() as usize, ROWS);

    // Time on X, by month. Reads the date column.
    let monthly = ChartRequest { x: axis(3, Role::Time), y: axis(2, Role::Continuous), statistic: Statistic::Mean, bucket: Some(TimeBucket::Month), ..ChartRequest::new(ChartKind::Bar) };
    let chart = timed("bar by month, first time (reads the dates)", || data.chart(&monthly).unwrap());
    assert_eq!(chart.rows_drawn as usize, ROWS);
    let XAxis::Bins { edges, .. } = &chart.x else { panic!("time is binned") };
    // 600,000 minutes is 416 days and 16 hours: January 2026 into February 2027.
    assert_eq!(edges.len() - 1, 14);
    timed("bar by month, again", || data.chart(&monthly).unwrap());

    // Scatter: too many points, so a grid of counts, then a sample, then a zoom that fits.
    let scatter = ChartRequest { x: axis(3, Role::Time), y: axis(2, Role::Continuous), ..ChartRequest::new(ChartKind::Scatter) };
    let chart = timed("scatter as a grid of counts", || data.chart(&scatter).unwrap());
    let density = chart.density.expect("a grid");
    assert_eq!(density.counts.iter().map(|c| *c as usize).sum::<usize>(), ROWS);
    let chart = timed("scatter as a sample", || data.chart(&ChartRequest { large: Large::Sample, ..scatter.clone() }).unwrap());
    assert!(chart.sampled);
    let XAxis::Bins { edges, .. } = &data.chart(&monthly).unwrap().x else { panic!() };
    let zoomed = ChartRequest { x_range: Some([edges[0], edges[0] + 3_600_000.0]), ..scatter.clone() };
    let chart = timed("scatter zoomed to one hour", || data.chart(&zoomed).unwrap());
    assert!(chart.density.is_none() && !chart.sampled);
    assert_eq!(chart.rows_drawn, 610, "ten rows a minute, for the 61 minutes from 00:00 to 01:00");

    // A trend line is fitted to every row, shaded or not.
    let trend = ChartRequest { trend: Some(TrendSpec { kind: TrendKind::Polynomial, degree: 3, per_group: false }), ..scatter.clone() };
    let chart = timed("scatter with a cubic trend line", || data.chart(&trend).unwrap());
    assert_eq!(chart.fits[0].rows as usize, ROWS);

    // A box for each region: every value is sorted.
    let boxes = ChartRequest { x: axis(1, Role::Discrete), y: axis(2, Role::Continuous), ..ChartRequest::new(ChartKind::Box) };
    let chart = timed("box plot by region", || data.chart(&boxes).unwrap());
    assert_eq!(chart.boxes.len(), REGIONS.len());
    assert_eq!(chart.boxes.iter().flatten().map(|b| b.rows as usize).sum::<usize>(), ROWS);

    // A control chart by day, and one of the rows in order.
    let by_day = ChartRequest { x: axis(3, Role::Time), y: axis(2, Role::Continuous), bucket: Some(TimeBucket::Day), ..ChartRequest::new(ChartKind::Control) };
    let chart = timed("control chart by day", || data.chart(&by_day).unwrap());
    assert_eq!(chart.series[0].values.len(), 417);
    assert_eq!(chart.stats.as_ref().unwrap().rows as usize, ROWS);
    let in_order = ChartRequest { y: axis(2, Role::Continuous), ..ChartRequest::new(ChartKind::Control) };
    let chart = timed("control chart of the rows in order", || data.chart(&in_order).unwrap());
    assert_eq!(chart.x, XAxis::Sequence { count: 2_000, rows_per_point: 3_000 });

    // The id column as categories: six million different values.
    let summary = timed("summary of a column with 6M different values", || data.summary(0, Role::Discrete).unwrap());
    assert!(summary.overflow);

    println!("kept columns: {} MB", data.kept_bytes() >> 20);
    assert_eq!(memory_in_use() - before, data.kept_bytes(), "kept columns are counted against the budget");
    drop(data);
    assert!(memory_in_use() < before, "and given back, with the result itself, when the chart is closed");
}
