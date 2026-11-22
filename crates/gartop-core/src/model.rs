//! Domain models shared by the fetcher and the viewer.

use chrono::{NaiveDate, NaiveDateTime};

/// A per-day data source. Each one maps to one Garmin range endpoint and is stored
/// raw (one JSON item per day) in `raw_daily`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// `/usersummary-service/v2/dailysummaries`: steps, calories, heart rate, stress, body battery.
    Daily,
    /// `/sleep-service/sleep/dailySleepSummariesByDate`: sleep score and stages.
    Sleep,
    /// `/hrv-service/hrv/daily/{start}/{end}`: overnight HRV and status.
    Hrv,
    /// `/metrics-service/metrics/maxmet/daily/{start}/{end}`: VO2 max estimates.
    Vo2Max,
}

impl Source {
    pub const ALL: [Self; 4] = [Self::Daily, Self::Sleep, Self::Hrv, Self::Vo2Max];

    /// Stable identifier stored in the database.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Sleep => "sleep",
            Self::Hrv => "hrv",
            Self::Vo2Max => "vo2max",
        }
    }

    #[must_use]
    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|src| src.as_str() == s)
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Everything known about one calendar day. Every metric is optional: the watch may not
/// have been worn, or the metric may not apply.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DailyMetrics {
    pub day: NaiveDate,
    // daily summary
    pub steps: Option<i64>,
    pub step_goal: Option<i64>,
    pub distance_m: Option<f64>,
    pub active_kcal: Option<f64>,
    pub resting_kcal: Option<f64>,
    pub total_kcal: Option<f64>,
    pub resting_hr: Option<i64>,
    pub min_hr: Option<i64>,
    pub max_hr: Option<i64>,
    pub stress_avg: Option<i64>,
    pub body_battery_max: Option<i64>,
    pub body_battery_min: Option<i64>,
    pub intensity_moderate: Option<i64>,
    pub intensity_vigorous: Option<i64>,
    // sleep
    pub sleep_score: Option<i64>,
    pub sleep_qualifier: Option<String>,
    pub sleep_seconds: Option<i64>,
    pub deep_sleep_seconds: Option<i64>,
    pub light_sleep_seconds: Option<i64>,
    pub rem_sleep_seconds: Option<i64>,
    pub awake_seconds: Option<i64>,
    // hrv
    pub hrv_last_night: Option<i64>,
    pub hrv_weekly_avg: Option<i64>,
    pub hrv_status: Option<String>,
    pub hrv_baseline_low: Option<i64>,
    pub hrv_baseline_high: Option<i64>,
    // vo2 max
    pub vo2max_running: Option<f64>,
    pub vo2max_cycling: Option<f64>,
}

/// One recorded activity (run, ride, walk, …).
#[derive(Debug, Clone, PartialEq)]
pub struct Activity {
    pub id: i64,
    pub start_local: NaiveDateTime,
    pub name: Option<String>,
    pub type_key: String,
    pub distance_m: Option<f64>,
    pub duration_s: Option<f64>,
    pub moving_s: Option<f64>,
    pub avg_hr: Option<f64>,
    pub max_hr: Option<f64>,
    pub avg_speed_mps: Option<f64>,
    pub calories: Option<f64>,
    pub elevation_gain_m: Option<f64>,
    pub aerobic_te: Option<f64>,
    pub training_load: Option<f64>,
}

impl Activity {
    /// Whether this is any kind of run (road, trail, treadmill, track, …).
    #[must_use]
    pub fn is_run(&self) -> bool {
        self.type_key.contains("running")
    }

    /// Average pace in seconds per kilometre, from moving time when available.
    #[must_use]
    pub fn pace_s_per_km(&self) -> Option<f64> {
        let dist = self.distance_m.filter(|d| *d > 0.0)?;
        let time = self.moving_s.or(self.duration_s)?;
        Some(time / (dist / 1000.0))
    }
}
