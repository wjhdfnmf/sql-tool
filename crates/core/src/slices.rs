//! Running one long query as many short ones.
//!
//! A query over a long stretch of time can run for longer than a server, a
//! proxy or a query timeout allows. If the query says where its time range
//! goes, with `{start}` and `{end}`, the range can be cut into slices and the
//! query run once for each, the rows of every slice put end to end in the
//! order of the slices. Each run is short, so none times out, and several
//! can run at once on connections of their own.
//!
//! The slices are half-open: a query should keep rows from `{start}` up to
//! but not including `{end}` (`t >= {start} AND t < {end}`). With `BETWEEN`,
//! which includes both ends, a row exactly on a boundary is returned by two
//! slices.

use crate::chart::{civil_from_days, parse_time};
use crate::error::Error;
use crate::guard::{self, StatementKind};

/// More slices than this is almost certainly a mistake in the interval.
pub const MAX_SLICES: usize = 5_000;

/// One run of the query: the stretch of time it covers, and its SQL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slice {
    pub start: String,
    pub end: String,
    pub sql: String,
}

fn refuse<T>(message: impl Into<String>) -> Result<T, Error> {
    Err(Error::Other(message.into()))
}

/// Reads `2026-07-04`, `2026-07-04 13:30` or `2026-07-04 13:30:15` as whole seconds since 1970.
fn seconds(text: &str, what: &str) -> Result<i64, Error> {
    match parse_time(text.as_bytes()) {
        Some((ms, true)) => Ok((ms / 1000.0).floor() as i64),
        _ => refuse(format!("The {what} of the range, \"{}\", is not a date and time such as 2026-07-04 13:30.", text.trim())),
    }
}

/// Seconds since 1970 as `YYYY-MM-DD HH:MM:SS`.
fn stamp(seconds: i64) -> String {
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}", rest / 3600, rest / 60 % 60, rest % 60)
}

/// The placeholders a query can mark its time range with. `{0}` and `{1}`
/// are accepted too, as scripts that fill a query with `format` write them.
const STARTS: [&str; 2] = ["{start}", "{0}"];
const ENDS: [&str; 2] = ["{end}", "{1}"];

/// Whether the query marks where its time range goes.
pub fn has_placeholders(sql: &str) -> bool {
    let lower = sql.to_ascii_lowercase();
    STARTS.iter().any(|p| lower.contains(p)) && ENDS.iter().any(|p| lower.contains(p))
}

/// Puts a time, as a quoted literal, wherever one of the placeholders is. A
/// placeholder that is already inside quotes keeps them and gets no second pair.
fn fill(sql: &str, placeholders: &[&str], value: &str) -> String {
    let lower = sql.to_ascii_lowercase();
    let mut out = String::with_capacity(sql.len() + 32);
    let mut at = 0;
    while at < sql.len() {
        match placeholders.iter().find(|p| lower[at..].starts_with(**p)) {
            Some(placeholder) => {
                let quoted = out.ends_with('\'') && sql[at + placeholder.len()..].starts_with('\'');
                if quoted {
                    out.push_str(value);
                } else {
                    out.push('\'');
                    out.push_str(value);
                    out.push('\'');
                }
                at += placeholder.len();
            }
            None => {
                let next = sql[at..].chars().next().expect("not at the end");
                out.push(next);
                at += next.len_utf8();
            }
        }
    }
    out
}

/// The query over the whole range at once: what the slices add up to. Kept
/// with the result, so that it can be run again as one statement.
pub fn whole(sql: &str, start: &str, end: &str) -> Result<String, Error> {
    let (from, to) = (seconds(start, "start")?, seconds(end, "end")?);
    Ok(fill(&fill(sql, &STARTS, &stamp(from)), &ENDS, &stamp(to)))
}

/// Cuts `start..end` into slices of `step_secs` and writes the query for
/// each. The last slice ends at `end`, however short that makes it.
///
/// Refused: a query without both placeholders, a range that is empty or
/// backwards, more than [`MAX_SLICES`] slices, and anything that is not a
/// plain read, since a statement that changes something must not be run
/// many times over.
pub fn plan(sql: &str, start: &str, end: &str, step_secs: u64) -> Result<Vec<Slice>, Error> {
    if !has_placeholders(sql) {
        return refuse(
            "To run a query in time slices, mark where its time range goes with {start} and {end}, \
             for example: WHERE created >= {start} AND created < {end}",
        );
    }
    let (from, to) = (seconds(start, "start")?, seconds(end, "end")?);
    if to <= from {
        return refuse("The end of the range must be after its start.");
    }
    if step_secs == 0 {
        return refuse("The length of a slice must be at least one second.");
    }
    let count = ((to - from) as u64).div_ceil(step_secs);
    if count as usize > MAX_SLICES {
        return refuse(format!("That is {count} slices, more than the {MAX_SLICES} allowed. Use longer slices or a shorter range."));
    }
    let mut slices = Vec::with_capacity(count as usize);
    let mut at = from;
    while at < to {
        let next = (at + step_secs as i64).min(to);
        let (first, last) = (stamp(at), stamp(next));
        let text = fill(&fill(sql, &STARTS, &first), &ENDS, &last);
        slices.push(Slice { start: first, end: last, sql: text });
        at = next;
    }
    if guard::classify(&slices[0].sql) != StatementKind::Read {
        return refuse("Only a SELECT or another plain read can be run in time slices: a statement that changes something would run once for every slice.");
    }
    Ok(slices)
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUERY: &str = "SELECT * FROM events WHERE at >= {start} AND at < {end}";

    #[test]
    fn a_range_is_cut_into_slices_that_meet_end_to_end() {
        let slices = plan(QUERY, "2026-09-18 22:00", "2026-09-19 01:30:00", 3600).unwrap();
        let spans: Vec<(&str, &str)> = slices.iter().map(|s| (s.start.as_str(), s.end.as_str())).collect();
        assert_eq!(
            spans,
            [
                ("2026-09-18 22:00:00", "2026-09-18 23:00:00"),
                ("2026-09-18 23:00:00", "2026-09-19 00:00:00"),
                ("2026-09-19 00:00:00", "2026-09-19 01:00:00"),
                // The last one stops at the end of the range, not a full hour on.
                ("2026-09-19 01:00:00", "2026-09-19 01:30:00"),
            ]
        );
        assert_eq!(slices[0].sql, "SELECT * FROM events WHERE at >= '2026-09-18 22:00:00' AND at < '2026-09-18 23:00:00'");
        assert_eq!(
            whole(QUERY, "2026-09-18 22:00", "2026-09-19 01:30").unwrap(),
            "SELECT * FROM events WHERE at >= '2026-09-18 22:00:00' AND at < '2026-09-19 01:30:00'"
        );
        // Each slice starts where the one before ended, across month and year ends too.
        let year_end = plan(QUERY, "2026-12-31", "2027-01-02", 86_400).unwrap();
        assert_eq!(year_end[1].start, "2027-01-01 00:00:00");
        assert_eq!(plan(QUERY, "2024-02-28", "2024-03-01", 86_400).unwrap().len(), 2, "2024 has a 29 February");
    }

    #[test]
    fn placeholders_are_found_however_they_are_written() {
        // As a script that fills the query with format() numbers them.
        let numbered = plan("SELECT 1 FROM t WHERE c BETWEEN {0} AND {1}", "2026-01-01", "2026-01-02", 86_400).unwrap();
        assert_eq!(numbered[0].sql, "SELECT 1 FROM t WHERE c BETWEEN '2026-01-01 00:00:00' AND '2026-01-02 00:00:00'");
        // Already in quotes, in capitals, and used twice.
        let quoted = plan("SELECT '{START}' AS s FROM t WHERE c >= '{start}' AND c < '{End}'", "2026-01-01", "2026-01-02", 86_400).unwrap();
        assert_eq!(
            quoted[0].sql,
            "SELECT '2026-01-01 00:00:00' AS s FROM t WHERE c >= '2026-01-01 00:00:00' AND c < '2026-01-02 00:00:00'"
        );
        // Text that is not plain ASCII is left as it is.
        let accented = plan("SELECT 'Zoë' FROM t WHERE c >= {start} AND c < {end}", "2026-01-01", "2026-01-02", 86_400).unwrap();
        assert!(accented[0].sql.starts_with("SELECT 'Zoë' FROM t"));
        assert!(has_placeholders(QUERY) && !has_placeholders("SELECT {start}") && !has_placeholders("SELECT 1"));
    }

    #[test]
    fn what_cannot_be_sliced_is_refused_with_the_reason() {
        let refused = |sql: &str, start: &str, end: &str, step: u64| plan(sql, start, end, step).unwrap_err().to_string();
        assert!(refused("SELECT 1", "2026-01-01", "2026-01-02", 3600).contains("{start} and {end}"));
        assert!(refused(QUERY, "yesterday", "2026-01-02", 3600).contains("start of the range"));
        assert!(refused(QUERY, "2026-01-01", "12:00", 3600).contains("end of the range"));
        assert!(refused(QUERY, "2026-01-02", "2026-01-01", 3600).contains("after its start"));
        assert!(refused(QUERY, "2026-01-01", "2026-01-01", 3600).contains("after its start"));
        assert!(refused(QUERY, "2026-01-01", "2026-01-02", 0).contains("at least one second"));
        assert!(refused(QUERY, "2020-01-01", "2026-01-01", 60).contains("more than the 5000 allowed"));
        // A change must never run once per slice.
        let delete = "DELETE FROM events WHERE at >= {start} AND at < {end}";
        assert!(refused(delete, "2026-01-01", "2026-01-02", 3600).contains("plain read"));
    }
}
