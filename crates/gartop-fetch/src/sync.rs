//! Fetching date ranges from the API into the store.

use chrono::{Days, NaiveDate};
use gartop_core::Store;
use gartop_core::model::Source;
use serde_json::Value;

use crate::client::{ApiError, GarminClient};

/// Page size for the activity search endpoint.
const ACTIVITY_PAGE: usize = 100;

/// Largest range (in days) each source accepts per request. Sleep rejects 90-day
/// ranges with HTTP 400; the others accept at least 90.
const fn chunk_days(source: Source) -> u64 {
    match source {
        Source::Sleep => 28,
        Source::Daily | Source::Hrv | Source::Vo2Max => 90,
    }
}

fn day(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// The request for one source over `from..=to`.
fn request(
    source: Source,
    from: NaiveDate,
    to: NaiveDate,
) -> (String, Vec<(&'static str, String)>) {
    match source {
        Source::Daily => (
            "/usersummary-service/v2/dailysummaries".into(),
            vec![("startDate", day(from)), ("endDate", day(to))],
        ),
        Source::Sleep => (
            "/sleep-service/sleep/dailySleepSummariesByDate".into(),
            vec![("startDate", day(from)), ("endDate", day(to))],
        ),
        Source::Hrv => (
            format!("/hrv-service/hrv/daily/{}/{}", day(from), day(to)),
            vec![],
        ),
        Source::Vo2Max => (
            format!(
                "/metrics-service/metrics/maxmet/daily/{}/{}",
                day(from),
                day(to)
            ),
            vec![],
        ),
    }
}

/// Split `from..=to` into consecutive inclusive chunks of at most `len` days.
fn chunks(from: NaiveDate, to: NaiveDate, len: u64) -> Vec<(NaiveDate, NaiveDate)> {
    let mut out = Vec::new();
    let mut start = from;
    while start <= to {
        let end = start
            .checked_add_days(Days::new(len - 1))
            .map_or(to, |e| e.min(to));
        out.push((start, end));
        match end.succ_opt() {
            Some(next) => start = next,
            None => break,
        }
    }
    out
}

/// Options for one sync run.
#[derive(Debug, Clone, Copy)]
pub struct SyncOptions {
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// Skip chunks whose every day is already stored (used by backfill).
    pub skip_existing: bool,
}

/// What a sync run did.
#[derive(Debug, Default)]
pub struct SyncReport {
    pub days_stored: usize,
    pub activities_stored: usize,
    pub chunks_skipped: usize,
    pub failures: Vec<String>,
}

/// Fetch every source and the activity list for the given range.
pub fn run(client: &mut GarminClient, store: &mut Store, opts: SyncOptions) -> SyncReport {
    let mut report = SyncReport::default();
    for source in Source::ALL {
        for (from, to) in chunks(opts.from, opts.to, chunk_days(source)) {
            if opts.skip_existing && chunk_stored(store, source, from, to) {
                report.chunks_skipped += 1;
                continue;
            }
            match fetch_chunk(client, store, source, from, to) {
                Ok(n) => {
                    tracing::info!("{source:>7} {from}..{to}: {n} days");
                    report.days_stored += n;
                }
                Err(e) => {
                    tracing::error!("{source} {from}..{to}: {e:#}");
                    report
                        .failures
                        .push(format!("{source} {from}..{to}: {e:#}"));
                }
            }
        }
    }
    match fetch_activities(client, store, opts.from, opts.to) {
        Ok(n) => {
            tracing::info!("activities {}..{}: {n}", opts.from, opts.to);
            report.activities_stored = n;
        }
        Err(e) => {
            tracing::error!("activities: {e:#}");
            report.failures.push(format!("activities: {e:#}"));
        }
    }
    report
}

fn chunk_stored(store: &Store, source: Source, from: NaiveDate, to: NaiveDate) -> bool {
    from.iter_days()
        .take_while(|d| *d <= to)
        .all(|d| store.has_day(source, d).unwrap_or(false))
}

fn fetch_chunk(
    client: &mut GarminClient,
    store: &mut Store,
    source: Source,
    from: NaiveDate,
    to: NaiveDate,
) -> anyhow::Result<usize> {
    let (path, query) = request(source, from, to);
    let body = match client.get_json(&path, &query) {
        Ok(v) => v,
        // No data at all for the range.
        Err(e) if matches!(e.status(), Some(204 | 404)) => return Ok(0),
        Err(e) => return Err(e.into()),
    };
    Ok(store.put_range(source, &body)?)
}

fn fetch_activities(
    client: &mut GarminClient,
    store: &mut Store,
    from: NaiveDate,
    to: NaiveDate,
) -> anyhow::Result<usize> {
    let mut total = 0;
    let mut start = 0usize;
    loop {
        let query = [
            ("start", start.to_string()),
            ("limit", ACTIVITY_PAGE.to_string()),
            ("startDate", day(from)),
            ("endDate", day(to)),
        ];
        let page =
            match client.get_json("/activitylist-service/activities/search/activities", &query) {
                Ok(Value::Array(items)) => items,
                Ok(Value::Null) | Err(ApiError::Status { status: 404, .. }) => Vec::new(),
                Ok(other) => anyhow::bail!("activity search returned non-array: {other:.100}"),
                Err(e) => return Err(e.into()),
            };
        total += store.put_activities(&page)?;
        if page.len() < ACTIVITY_PAGE {
            return Ok(total);
        }
        start += page.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn chunking_covers_range_exactly() {
        let c = chunks(d("2026-01-01"), d("2026-03-01"), 28);
        assert_eq!(c.first().unwrap().0, d("2026-01-01"));
        assert_eq!(c.last().unwrap().1, d("2026-03-01"));
        for w in c.windows(2) {
            assert_eq!(w[0].1.succ_opt().unwrap(), w[1].0);
        }
        assert!(c.iter().all(|(a, b)| (*b - *a).num_days() < 28));
        assert_eq!(chunks(d("2026-01-01"), d("2026-01-01"), 90).len(), 1);
        assert!(chunks(d("2026-01-02"), d("2026-01-01"), 90).is_empty());
    }

    #[test]
    fn requests_use_expected_shapes() {
        let (p, q) = request(Source::Sleep, d("2026-01-01"), d("2026-01-07"));
        assert_eq!(p, "/sleep-service/sleep/dailySleepSummariesByDate");
        assert_eq!(q[0], ("startDate", "2026-01-01".to_owned()));
        let (p, q) = request(Source::Hrv, d("2026-01-01"), d("2026-01-07"));
        assert_eq!(p, "/hrv-service/hrv/daily/2026-01-01/2026-01-07");
        assert!(q.is_empty());
    }
}
