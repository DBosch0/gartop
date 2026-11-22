//! Small aggregations over stored metrics used by the dashboard.

use chrono::{Datelike as _, Days, NaiveDate};
use gartop_core::model::{Activity, DailyMetrics};

/// A metric accessor.
pub type Metric = fn(&DailyMetrics) -> Option<f64>;

/// The most recent day (on or before `today`) with a value, and that value.
pub fn latest(days: &[DailyMetrics], today: NaiveDate, f: Metric) -> Option<(NaiveDate, f64)> {
    days.iter()
        .rev()
        .filter(|d| d.day <= today)
        .find_map(|d| f(d).map(|v| (d.day, v)))
}

/// Mean of a metric over `from..=to`, ignoring missing days.
pub fn mean(days: &[DailyMetrics], from: NaiveDate, to: NaiveDate, f: Metric) -> Option<f64> {
    let vals: Vec<f64> = days
        .iter()
        .filter(|d| d.day >= from && d.day <= to)
        .filter_map(f)
        .collect();
    (!vals.is_empty()).then(|| vals.iter().sum::<f64>() / vals.len() as f64)
}

/// Value on `day` compared with the mean of the seven days before it.
pub fn delta_vs_week(days: &[DailyMetrics], day: NaiveDate, value: f64, f: Metric) -> Option<f64> {
    let to = day.pred_opt()?;
    let from = day.checked_sub_days(Days::new(7))?;
    mean(days, from, to, f).map(|m| value - m)
}

/// `(x, y)` points for a chart, with `x` = days since `from`.
pub fn series(days: &[DailyMetrics], from: NaiveDate, to: NaiveDate, f: Metric) -> Vec<(f64, f64)> {
    days.iter()
        .filter(|d| d.day >= from && d.day <= to)
        .filter_map(|d| f(d).map(|v| ((d.day - from).num_days() as f64, v)))
        .collect()
}

/// Like [`series`], but each day without a value repeats the previous value (for
/// slowly-changing estimates like VO2 max that are only reported on some days).
pub fn carried_series(
    days: &[DailyMetrics],
    from: NaiveDate,
    to: NaiveDate,
    f: Metric,
) -> Vec<(f64, f64)> {
    let mut last = days
        .iter()
        .filter(|d| d.day < from)
        .filter_map(f)
        .next_back();
    let mut out = Vec::new();
    for d in days.iter().filter(|d| d.day >= from && d.day <= to) {
        if let Some(v) = f(d) {
            last = Some(v);
        }
        if let Some(v) = last {
            out.push(((d.day - from).num_days() as f64, v));
        }
    }
    out
}

/// Monday of the week containing `d`.
pub fn week_start(d: NaiveDate) -> NaiveDate {
    d - Days::new(u64::from(d.weekday().num_days_from_monday()))
}

/// Running distance (km) per week for the `weeks` weeks ending with the week of `today`.
pub fn weekly_run_km(acts: &[Activity], today: NaiveDate, weeks: usize) -> Vec<(NaiveDate, f64)> {
    let last = week_start(today);
    let first = last - Days::new(7 * (weeks.max(1) as u64 - 1));
    let mut out: Vec<(NaiveDate, f64)> = (0..weeks.max(1))
        .map(|i| (first + Days::new(7 * i as u64), 0.0))
        .collect();
    for a in acts.iter().filter(|a| a.is_run()) {
        let ws = week_start(a.start_local.date());
        if ws >= first && ws <= last {
            let idx = ((ws - first).num_days() / 7) as usize;
            out[idx].1 += a.distance_m.unwrap_or(0.0) / 1000.0;
        }
    }
    out
}

/// Summary of the runs in a period.
#[derive(Debug, Default, PartialEq)]
pub struct RunSummary {
    pub count: usize,
    pub km: f64,
    pub moving_s: f64,
    pub longest_km: f64,
    pub avg_hr: Option<f64>,
}

impl RunSummary {
    pub fn pace_s_per_km(&self) -> Option<f64> {
        (self.km > 0.0).then(|| self.moving_s / self.km)
    }
}

pub fn summarize_runs<'a>(runs: impl IntoIterator<Item = &'a Activity>) -> RunSummary {
    let mut s = RunSummary::default();
    let (mut hr_sum, mut hr_n) = (0.0, 0usize);
    for a in runs.into_iter().filter(|a| a.is_run()) {
        let km = a.distance_m.unwrap_or(0.0) / 1000.0;
        s.count += 1;
        s.km += km;
        s.moving_s += a.moving_s.or(a.duration_s).unwrap_or(0.0);
        s.longest_km = s.longest_km.max(km);
        if let Some(hr) = a.avg_hr {
            hr_sum += hr;
            hr_n += 1;
        }
    }
    s.avg_hr = (hr_n > 0).then(|| hr_sum / hr_n as f64);
    s
}

/// `m:ss` for a pace or short duration in seconds.
pub fn fmt_min_sec(secs: f64) -> String {
    let total = secs.round() as i64;
    format!("{}:{:02}", total / 60, total % 60)
}

/// `h:mm:ss` or `m:ss` for a duration in seconds.
pub fn fmt_duration(secs: f64) -> String {
    let total = secs.round() as i64;
    if total >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            total / 3600,
            (total % 3600) / 60,
            total % 60
        )
    } else {
        fmt_min_sec(secs)
    }
}

/// `7h12m` for a sleep duration in seconds.
pub fn fmt_hours(secs: f64) -> String {
    let mins = (secs / 60.0).round() as i64;
    format!("{}h{:02}m", mins / 60, mins % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDateTime;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn day(s: &str, rhr: Option<i64>, vo2: Option<f64>) -> DailyMetrics {
        DailyMetrics {
            day: d(s),
            resting_hr: rhr,
            vo2max_running: vo2,
            ..DailyMetrics::default()
        }
    }

    fn rhr(m: &DailyMetrics) -> Option<f64> {
        m.resting_hr.map(|v| v as f64)
    }

    fn run(date: &str, km: f64, secs: f64, kind: &str) -> Activity {
        Activity {
            id: 0,
            start_local: NaiveDateTime::parse_from_str(
                &format!("{date} 08:00:00"),
                "%Y-%m-%d %H:%M:%S",
            )
            .unwrap(),
            name: None,
            type_key: kind.into(),
            distance_m: Some(km * 1000.0),
            duration_s: Some(secs),
            moving_s: None,
            avg_hr: Some(150.0),
            max_hr: None,
            avg_speed_mps: None,
            calories: None,
            elevation_gain_m: None,
            aerobic_te: None,
            training_load: None,
        }
    }

    #[test]
    fn latest_mean_delta() {
        let days: Vec<_> = (1..=8)
            .map(|i| day(&format!("2026-01-{i:02}"), Some(50 + i), None))
            .chain([day("2026-01-09", None, None)])
            .collect();
        assert_eq!(
            latest(&days, d("2026-01-09"), rhr),
            Some((d("2026-01-08"), 58.0))
        );
        assert_eq!(
            mean(&days, d("2026-01-01"), d("2026-01-07"), rhr),
            Some(54.0)
        );
        assert_eq!(delta_vs_week(&days, d("2026-01-08"), 58.0, rhr), Some(4.0));
        assert_eq!(mean(&days, d("2026-02-01"), d("2026-02-07"), rhr), None);
    }

    #[test]
    fn carry_forward_fills_gaps() {
        let vo2 = |m: &DailyMetrics| m.vo2max_running;
        let days = vec![
            day("2026-01-01", None, Some(40.0)),
            day("2026-01-03", None, None),
            day("2026-01-04", None, Some(41.0)),
        ];
        let s = carried_series(&days, d("2026-01-02"), d("2026-01-04"), vo2);
        assert_eq!(s, vec![(1.0, 40.0), (2.0, 41.0)]);
        assert_eq!(
            series(&days, d("2026-01-02"), d("2026-01-04"), vo2),
            vec![(2.0, 41.0)]
        );
    }

    #[test]
    fn weekly_buckets_and_summary() {
        // 2026-01-05 is a Monday.
        let acts = vec![
            run("2026-01-05", 5.0, 1500.0, "running"),
            run("2026-01-07", 10.0, 3000.0, "treadmill_running"),
            run("2026-01-13", 8.0, 2400.0, "running"),
            run("2026-01-13", 30.0, 3600.0, "cycling"),
        ];
        let w = weekly_run_km(&acts, d("2026-01-14"), 3);
        assert_eq!(
            w.iter().map(|x| x.0).collect::<Vec<_>>(),
            [d("2025-12-29"), d("2026-01-05"), d("2026-01-12")]
        );
        assert_eq!(w.iter().map(|x| x.1).collect::<Vec<_>>(), [0.0, 15.0, 8.0]);
        let s = summarize_runs(&acts);
        assert_eq!(s.count, 3);
        assert!((s.km - 23.0).abs() < 1e-9);
        assert_eq!(s.longest_km, 10.0);
        assert_eq!(s.pace_s_per_km(), Some(300.0));
    }

    #[test]
    fn formatting() {
        assert_eq!(fmt_min_sec(305.4), "5:05");
        assert_eq!(fmt_duration(3725.0), "1:02:05");
        assert_eq!(fmt_hours(26_100.0), "7h15m");
    }
}
