//! SQLite storage.
//!
//! Raw API items are the source of truth (`raw_daily`, `activities.raw`); the typed
//! `daily_metrics` columns are derived from them and can be rebuilt with [`Store::reparse`].

use std::path::Path;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use rusqlite::{Connection, OptionalExtension as _, Row, params};
use serde_json::Value;

use crate::model::{Activity, DailyMetrics, Source};
use crate::parse::{self, DayPatch, ParseError};

/// Errors from the storage layer.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("could not create {path}: {source}")]
    CreateDir {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error("database schema version {0} is newer than this build supports")]
    TooNew(i64),
}

/// Ordered schema migrations; `PRAGMA user_version` records how many have run.
const MIGRATIONS: &[&str] = &[r"
CREATE TABLE raw_daily (
    source     TEXT NOT NULL,
    day        TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    body       TEXT NOT NULL,
    PRIMARY KEY (source, day)
) STRICT;

CREATE TABLE daily_metrics (
    day                 TEXT PRIMARY KEY,
    steps               INTEGER,
    step_goal           INTEGER,
    distance_m          REAL,
    active_kcal         REAL,
    resting_kcal        REAL,
    total_kcal          REAL,
    resting_hr          INTEGER,
    min_hr              INTEGER,
    max_hr              INTEGER,
    stress_avg          INTEGER,
    body_battery_max    INTEGER,
    body_battery_min    INTEGER,
    intensity_moderate  INTEGER,
    intensity_vigorous  INTEGER,
    sleep_score         INTEGER,
    sleep_qualifier     TEXT,
    sleep_seconds       INTEGER,
    deep_sleep_seconds  INTEGER,
    light_sleep_seconds INTEGER,
    rem_sleep_seconds   INTEGER,
    awake_seconds       INTEGER,
    hrv_last_night      INTEGER,
    hrv_weekly_avg      INTEGER,
    hrv_status          TEXT,
    hrv_baseline_low    INTEGER,
    hrv_baseline_high   INTEGER,
    vo2max_running      REAL,
    vo2max_cycling      REAL
) STRICT;

CREATE TABLE activities (
    activity_id      INTEGER PRIMARY KEY,
    start_local      TEXT NOT NULL,
    name             TEXT,
    type_key         TEXT NOT NULL,
    distance_m       REAL,
    duration_s       REAL,
    moving_s         REAL,
    avg_hr           REAL,
    max_hr           REAL,
    avg_speed_mps    REAL,
    calories         REAL,
    elevation_gain_m REAL,
    aerobic_te       REAL,
    training_load    REAL,
    fetched_at       TEXT NOT NULL,
    raw              TEXT NOT NULL
) STRICT;
CREATE INDEX activities_start ON activities (start_local);

CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;
"];

const DATE_FMT: &str = "%Y-%m-%d";
const DATETIME_FMT: &str = "%Y-%m-%d %H:%M:%S";
const LAST_SYNC_KEY: &str = "last_sync";

/// Handle to the gartop database.
#[derive(Debug)]
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (creating if needed) the database at `path` and run migrations.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|source| StoreError::CreateDir {
                path: dir.to_path_buf(),
                source,
            })?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        Self::init(conn)
    }

    /// A private in-memory database (for tests).
    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "foreign_keys", true)?;
        let mut store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<(), StoreError> {
        let version: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?;
        let target = MIGRATIONS.len() as i64;
        if version > target {
            return Err(StoreError::TooNew(version));
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", i as i64 + 1)?;
            tx.commit()?;
        }
        Ok(())
    }

    // ---------------------------------------------------------------- writes

    /// Store a whole range response for `source`, returning how many days it held.
    pub fn put_range(&mut self, source: Source, body: &Value) -> Result<usize, StoreError> {
        let days = parse::split_days(source, body)?;
        let now = now_str();
        let tx = self.conn.transaction()?;
        for (day, item) in &days {
            put_day(&tx, source, *day, item, &now)?;
        }
        tx.commit()?;
        Ok(days.len())
    }

    /// Store activity-search items, returning how many were stored.
    pub fn put_activities(&mut self, items: &[Value]) -> Result<usize, StoreError> {
        let now = now_str();
        let tx = self.conn.transaction()?;
        let mut n = 0;
        for item in items {
            match parse::parse_activity(item) {
                Ok(a) => {
                    upsert_activity(&tx, &a, item, &now)?;
                    n += 1;
                }
                Err(e) => tracing::warn!("skipping unparsable activity: {e}"),
            }
        }
        tx.commit()?;
        Ok(n)
    }

    /// Rebuild all derived tables from the stored raw JSON. Returns rows reparsed.
    pub fn reparse(&mut self) -> Result<usize, StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM daily_metrics", [])?;
        let raws: Vec<(String, String, String)> = {
            let mut stmt = tx.prepare("SELECT source, day, body FROM raw_daily ORDER BY day")?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<_, _>>()?
        };
        let mut n = 0;
        for (source, day, body) in raws {
            let (Some(source), Ok(day), Ok(item)) = (
                Source::from_name(&source),
                NaiveDate::parse_from_str(&day, DATE_FMT),
                serde_json::from_str::<Value>(&body),
            ) else {
                tracing::warn!("skipping malformed raw row {source}/{day}");
                continue;
            };
            apply_patch(&tx, day, &parse::parse_day(source, &item)?)?;
            n += 1;
        }
        let acts: Vec<(String, String)> = {
            let mut stmt = tx.prepare("SELECT raw, fetched_at FROM activities")?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?
        };
        for (raw, fetched_at) in acts {
            let Ok(item) = serde_json::from_str::<Value>(&raw) else {
                continue;
            };
            if let Ok(a) = parse::parse_activity(&item) {
                upsert_activity(&tx, &a, &item, &fetched_at)?;
                n += 1;
            }
        }
        tx.commit()?;
        Ok(n)
    }

    /// Record the time of a successful sync.
    pub fn set_last_sync(&self, at: DateTime<Utc>) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![LAST_SYNC_KEY, at.to_rfc3339()],
        )?;
        Ok(())
    }

    // ----------------------------------------------------------------- reads

    /// A counter that changes whenever *another* connection commits to the database.
    /// Cheap to poll; the viewer uses it to reload when the fetcher writes.
    pub fn data_version(&self) -> Result<i64, StoreError> {
        Ok(self
            .conn
            .pragma_query_value(None, "data_version", |r| r.get(0))?)
    }

    /// Time of the last successful sync, if any.
    pub fn last_sync(&self) -> Result<Option<DateTime<Utc>>, StoreError> {
        let v: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                [LAST_SYNC_KEY],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|d| d.with_timezone(&Utc)))
    }

    /// Whether a raw item for `source` on `day` is stored.
    pub fn has_day(&self, source: Source, day: NaiveDate) -> Result<bool, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM raw_daily WHERE source = ?1 AND day = ?2",
                params![source.as_str(), day.format(DATE_FMT).to_string()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Daily metrics for `from..=to`, oldest first. Days without data are absent.
    pub fn daily_range(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<DailyMetrics>, StoreError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT * FROM daily_metrics WHERE day BETWEEN ?1 AND ?2 ORDER BY day",
        )?;
        let rows = stmt.query_map(params![fmt_day(from), fmt_day(to)], daily_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Activities started within `from..=to` (local dates), oldest first.
    pub fn activities(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<Activity>, StoreError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT * FROM activities WHERE start_local >= ?1 AND start_local < ?2
             ORDER BY start_local",
        )?;
        let end = to.succ_opt().unwrap_or(to);
        let rows = stmt.query_map(params![fmt_day(from), fmt_day(end)], activity_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Start time of the most recent stored activity.
    pub fn latest_activity_start(&self) -> Result<Option<NaiveDateTime>, StoreError> {
        let v: Option<String> =
            self.conn
                .query_row("SELECT MAX(start_local) FROM activities", [], |r| r.get(0))?;
        Ok(v.and_then(|s| NaiveDateTime::parse_from_str(&s, DATETIME_FMT).ok()))
    }
}

fn now_str() -> String {
    Utc::now().to_rfc3339()
}

fn fmt_day(d: NaiveDate) -> String {
    d.format(DATE_FMT).to_string()
}

fn put_day(
    conn: &Connection,
    source: Source,
    day: NaiveDate,
    item: &Value,
    now: &str,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO raw_daily (source, day, fetched_at, body) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (source, day) DO UPDATE SET fetched_at = excluded.fetched_at, body = excluded.body",
        params![source.as_str(), fmt_day(day), now, item.to_string()],
    )?;
    apply_patch(conn, day, &parse::parse_day(source, item)?)
}

/// Upsert the columns one source owns for `day`, leaving other sources' columns alone.
fn apply_patch(conn: &Connection, day: NaiveDate, patch: &DayPatch) -> Result<(), StoreError> {
    let day = fmt_day(day);
    match patch {
        DayPatch::Daily(s) => conn.execute(
            "INSERT INTO daily_metrics (day, steps, step_goal, distance_m, active_kcal, resting_kcal,
                total_kcal, resting_hr, min_hr, max_hr, stress_avg, body_battery_max,
                body_battery_min, intensity_moderate, intensity_vigorous)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT (day) DO UPDATE SET
                steps = excluded.steps, step_goal = excluded.step_goal,
                distance_m = excluded.distance_m, active_kcal = excluded.active_kcal,
                resting_kcal = excluded.resting_kcal, total_kcal = excluded.total_kcal,
                resting_hr = excluded.resting_hr, min_hr = excluded.min_hr,
                max_hr = excluded.max_hr, stress_avg = excluded.stress_avg,
                body_battery_max = excluded.body_battery_max,
                body_battery_min = excluded.body_battery_min,
                intensity_moderate = excluded.intensity_moderate,
                intensity_vigorous = excluded.intensity_vigorous",
            params![
                day,
                s.steps,
                s.step_goal,
                s.distance_m,
                s.active_kcal,
                s.resting_kcal,
                s.total_kcal,
                s.resting_hr,
                s.min_hr,
                s.max_hr,
                s.stress_avg,
                s.body_battery_max,
                s.body_battery_min,
                s.intensity_moderate,
                s.intensity_vigorous
            ],
        )?,
        DayPatch::Sleep(s) => conn.execute(
            "INSERT INTO daily_metrics (day, sleep_score, sleep_qualifier, sleep_seconds,
                deep_sleep_seconds, light_sleep_seconds, rem_sleep_seconds, awake_seconds)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (day) DO UPDATE SET
                sleep_score = excluded.sleep_score, sleep_qualifier = excluded.sleep_qualifier,
                sleep_seconds = excluded.sleep_seconds,
                deep_sleep_seconds = excluded.deep_sleep_seconds,
                light_sleep_seconds = excluded.light_sleep_seconds,
                rem_sleep_seconds = excluded.rem_sleep_seconds,
                awake_seconds = excluded.awake_seconds",
            params![
                day,
                s.score,
                s.qualifier,
                s.sleep_seconds,
                s.deep_seconds,
                s.light_seconds,
                s.rem_seconds,
                s.awake_seconds
            ],
        )?,
        DayPatch::Hrv(h) => conn.execute(
            "INSERT INTO daily_metrics (day, hrv_last_night, hrv_weekly_avg, hrv_status,
                hrv_baseline_low, hrv_baseline_high)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (day) DO UPDATE SET
                hrv_last_night = excluded.hrv_last_night, hrv_weekly_avg = excluded.hrv_weekly_avg,
                hrv_status = excluded.hrv_status, hrv_baseline_low = excluded.hrv_baseline_low,
                hrv_baseline_high = excluded.hrv_baseline_high",
            params![
                day,
                h.last_night_avg,
                h.weekly_avg,
                h.status,
                h.baseline_low,
                h.baseline_high
            ],
        )?,
        DayPatch::Vo2Max(v) => conn.execute(
            "INSERT INTO daily_metrics (day, vo2max_running, vo2max_cycling) VALUES (?1, ?2, ?3)
             ON CONFLICT (day) DO UPDATE SET
                vo2max_running = COALESCE(excluded.vo2max_running, vo2max_running),
                vo2max_cycling = COALESCE(excluded.vo2max_cycling, vo2max_cycling)",
            params![day, v.running, v.cycling],
        )?,
    };
    Ok(())
}

fn upsert_activity(
    conn: &Connection,
    a: &Activity,
    raw: &Value,
    now: &str,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO activities (activity_id, start_local, name, type_key, distance_m, duration_s,
            moving_s, avg_hr, max_hr, avg_speed_mps, calories, elevation_gain_m, aerobic_te,
            training_load, fetched_at, raw)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
         ON CONFLICT (activity_id) DO UPDATE SET
            start_local = excluded.start_local, name = excluded.name, type_key = excluded.type_key,
            distance_m = excluded.distance_m, duration_s = excluded.duration_s,
            moving_s = excluded.moving_s, avg_hr = excluded.avg_hr, max_hr = excluded.max_hr,
            avg_speed_mps = excluded.avg_speed_mps, calories = excluded.calories,
            elevation_gain_m = excluded.elevation_gain_m, aerobic_te = excluded.aerobic_te,
            training_load = excluded.training_load, fetched_at = excluded.fetched_at,
            raw = excluded.raw",
        params![
            a.id,
            a.start_local.format(DATETIME_FMT).to_string(),
            a.name,
            a.type_key,
            a.distance_m,
            a.duration_s,
            a.moving_s,
            a.avg_hr,
            a.max_hr,
            a.avg_speed_mps,
            a.calories,
            a.elevation_gain_m,
            a.aerobic_te,
            a.training_load,
            now,
            raw.to_string()
        ],
    )?;
    Ok(())
}

fn parse_col<T>(
    s: &str,
    f: impl FnOnce(&str) -> Result<T, chrono::ParseError>,
) -> rusqlite::Result<T> {
    f(s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn daily_from_row(r: &Row<'_>) -> rusqlite::Result<DailyMetrics> {
    let day: String = r.get("day")?;
    Ok(DailyMetrics {
        day: parse_col(&day, |s| NaiveDate::parse_from_str(s, DATE_FMT))?,
        steps: r.get("steps")?,
        step_goal: r.get("step_goal")?,
        distance_m: r.get("distance_m")?,
        active_kcal: r.get("active_kcal")?,
        resting_kcal: r.get("resting_kcal")?,
        total_kcal: r.get("total_kcal")?,
        resting_hr: r.get("resting_hr")?,
        min_hr: r.get("min_hr")?,
        max_hr: r.get("max_hr")?,
        stress_avg: r.get("stress_avg")?,
        body_battery_max: r.get("body_battery_max")?,
        body_battery_min: r.get("body_battery_min")?,
        intensity_moderate: r.get("intensity_moderate")?,
        intensity_vigorous: r.get("intensity_vigorous")?,
        sleep_score: r.get("sleep_score")?,
        sleep_qualifier: r.get("sleep_qualifier")?,
        sleep_seconds: r.get("sleep_seconds")?,
        deep_sleep_seconds: r.get("deep_sleep_seconds")?,
        light_sleep_seconds: r.get("light_sleep_seconds")?,
        rem_sleep_seconds: r.get("rem_sleep_seconds")?,
        awake_seconds: r.get("awake_seconds")?,
        hrv_last_night: r.get("hrv_last_night")?,
        hrv_weekly_avg: r.get("hrv_weekly_avg")?,
        hrv_status: r.get("hrv_status")?,
        hrv_baseline_low: r.get("hrv_baseline_low")?,
        hrv_baseline_high: r.get("hrv_baseline_high")?,
        vo2max_running: r.get("vo2max_running")?,
        vo2max_cycling: r.get("vo2max_cycling")?,
    })
}

fn activity_from_row(r: &Row<'_>) -> rusqlite::Result<Activity> {
    let start: String = r.get("start_local")?;
    Ok(Activity {
        id: r.get("activity_id")?,
        start_local: parse_col(&start, |s| NaiveDateTime::parse_from_str(s, DATETIME_FMT))?,
        name: r.get("name")?,
        type_key: r.get("type_key")?,
        distance_m: r.get("distance_m")?,
        duration_s: r.get("duration_s")?,
        moving_s: r.get("moving_s")?,
        avg_hr: r.get("avg_hr")?,
        max_hr: r.get("max_hr")?,
        avg_speed_mps: r.get("avg_speed_mps")?,
        calories: r.get("calories")?,
        elevation_gain_m: r.get("elevation_gain_m")?,
        aerobic_te: r.get("aerobic_te")?,
        training_load: r.get("training_load")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, DATE_FMT).unwrap()
    }

    fn fixture(name: &str) -> Value {
        let path = format!(
            "{}/../../tests/fixtures/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn loaded() -> Store {
        let mut s = Store::open_in_memory().unwrap();
        s.put_range(Source::Daily, &fixture("dailysummaries"))
            .unwrap();
        s.put_range(Source::Sleep, &fixture("sleep")).unwrap();
        s.put_range(Source::Hrv, &fixture("hrv")).unwrap();
        s.put_range(Source::Vo2Max, &fixture("vo2max")).unwrap();
        let acts = fixture("activities");
        s.put_activities(acts.as_array().unwrap()).unwrap();
        s
    }

    #[test]
    fn sources_merge_into_one_row() {
        let s = loaded();
        let days = s.daily_range(d("2026-01-01"), d("2026-01-31")).unwrap();
        assert_eq!(days.len(), 2);
        let first = &days[0];
        assert_eq!(first.steps, Some(9_120));
        assert_eq!(first.sleep_score, Some(82));
        assert_eq!(first.hrv_status.as_deref(), Some("BALANCED"));
        assert_eq!(first.vo2max_running, Some(48.5));
        assert_eq!(days[1].vo2max_cycling, Some(45.0));
    }

    #[test]
    fn upsert_replaces_only_own_columns() {
        let mut s = loaded();
        let newer = json!({"data": [{"calendarDate": "2026-01-10",
            "movement": {"steps": {"value": 10_000}}}]});
        s.put_range(Source::Daily, &newer).unwrap();
        let day = &s.daily_range(d("2026-01-10"), d("2026-01-10")).unwrap()[0];
        assert_eq!(day.steps, Some(10_000));
        assert_eq!(day.resting_hr, None, "daily columns are replaced as a unit");
        assert_eq!(day.sleep_score, Some(82), "other sources untouched");
    }

    #[test]
    fn reparse_rebuilds_from_raw() {
        let mut s = loaded();
        s.conn
            .execute("UPDATE daily_metrics SET steps = NULL", [])
            .unwrap();
        s.conn
            .execute("UPDATE activities SET avg_hr = NULL", [])
            .unwrap();
        let n = s.reparse().unwrap();
        assert_eq!(n, 2 + 2 + 1 + 2 + 2);
        let days = s.daily_range(d("2026-01-10"), d("2026-01-10")).unwrap();
        assert_eq!(days[0].steps, Some(9_120));
        let acts = s.activities(d("2026-01-10"), d("2026-01-10")).unwrap();
        assert_eq!(acts[0].avg_hr, Some(150.0));
    }

    #[test]
    fn activity_queries() {
        let s = loaded();
        let all = s.activities(d("2026-01-10"), d("2026-01-11")).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(
            s.activities(d("2026-01-11"), d("2026-01-11"))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            s.latest_activity_start().unwrap().unwrap().to_string(),
            "2026-01-11 18:00:00"
        );
    }

    #[test]
    fn meta_and_has_day() {
        let s = loaded();
        assert!(s.last_sync().unwrap().is_none());
        let now = Utc::now();
        s.set_last_sync(now).unwrap();
        assert_eq!(s.last_sync().unwrap().unwrap().timestamp(), now.timestamp());
        assert!(s.has_day(Source::Hrv, d("2026-01-10")).unwrap());
        assert!(!s.has_day(Source::Hrv, d("2026-01-11")).unwrap());
    }

    #[test]
    fn data_version_tracks_other_connections() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gartop.db");
        let reader = Store::open(&path).unwrap();
        let before = reader.data_version().unwrap();
        let mut writer = Store::open(&path).unwrap();
        writer.put_range(Source::Hrv, &fixture("hrv")).unwrap();
        assert_ne!(reader.data_version().unwrap(), before);
    }

    #[test]
    fn file_db_reopens_without_remigrating() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/gartop.db");
        {
            let mut s = Store::open(&path).unwrap();
            s.put_range(Source::Hrv, &fixture("hrv")).unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert!(s.has_day(Source::Hrv, d("2026-01-10")).unwrap());
    }
}
