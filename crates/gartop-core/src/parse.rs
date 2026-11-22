//! Parsers turning raw Garmin API JSON into domain models.
//!
//! Range endpoints return many days at once; [`split_days`] cuts a response into
//! one JSON item per day (which is what gets stored raw), and [`parse_day`] turns
//! a single item into a typed [`DayPatch`]. Every field is optional so that schema
//! drift on Garmin's side degrades to missing values rather than errors.

use chrono::{NaiveDate, NaiveDateTime};
use serde::Deserialize;
use serde_json::Value;

use crate::model::{Activity, Source};

/// Errors from parsing API JSON.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("unexpected {src} response shape: {detail}")]
    Shape { src: Source, detail: String },
    #[error("invalid {what}: {err}")]
    Json {
        what: &'static str,
        err: serde_json::Error,
    },
    #[error("activity has no parsable start time")]
    ActivityStart,
}

/// Split a range response into `(day, item)` pairs.
pub fn split_days(source: Source, body: &Value) -> Result<Vec<(NaiveDate, Value)>, ParseError> {
    let items = match source {
        Source::Daily => body.get("data"),
        Source::Hrv => body.get("hrvSummaries"),
        Source::Sleep | Source::Vo2Max => Some(body),
    };
    let items = match items {
        Some(Value::Array(items)) => items,
        // An empty range may come back as null / {} rather than [].
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Object(o)) if o.is_empty() => return Ok(Vec::new()),
        Some(other) => {
            return Err(ParseError::Shape {
                src: source,
                detail: format!("expected an array, got {}", type_name(other)),
            });
        }
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let day = day_of(source, item).ok_or_else(|| ParseError::Shape {
            src: source,
            detail: "item without calendarDate".into(),
        })?;
        out.push((day, item.clone()));
    }
    Ok(out)
}

fn day_of(source: Source, item: &Value) -> Option<NaiveDate> {
    let date = match source {
        Source::Vo2Max => ["generic", "cycling"]
            .iter()
            .find_map(|k| item.get(k)?.get("calendarDate")?.as_str()),
        _ => item.get("calendarDate")?.as_str(),
    }?;
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// The metrics one source contributes to a day.
#[derive(Debug, Clone, PartialEq)]
pub enum DayPatch {
    Daily(DailySummary),
    Sleep(SleepSummary),
    Hrv(HrvSummary),
    Vo2Max(Vo2Max),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DailySummary {
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
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SleepSummary {
    pub score: Option<i64>,
    pub qualifier: Option<String>,
    pub sleep_seconds: Option<i64>,
    pub deep_seconds: Option<i64>,
    pub light_seconds: Option<i64>,
    pub rem_seconds: Option<i64>,
    pub awake_seconds: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HrvSummary {
    pub last_night_avg: Option<i64>,
    pub weekly_avg: Option<i64>,
    pub status: Option<String>,
    pub baseline_low: Option<i64>,
    pub baseline_high: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Vo2Max {
    pub running: Option<f64>,
    pub cycling: Option<f64>,
}

/// Parse one stored day item.
pub fn parse_day(source: Source, item: &Value) -> Result<DayPatch, ParseError> {
    let json = |err| ParseError::Json {
        what: source.as_str(),
        err,
    };
    Ok(match source {
        Source::Daily => DayPatch::Daily(
            serde_json::from_value::<raw::Daily>(item.clone())
                .map_err(json)?
                .into(),
        ),
        Source::Sleep => DayPatch::Sleep(
            serde_json::from_value::<raw::Sleep>(item.clone())
                .map_err(json)?
                .into(),
        ),
        Source::Hrv => DayPatch::Hrv(
            serde_json::from_value::<raw::Hrv>(item.clone())
                .map_err(json)?
                .into(),
        ),
        Source::Vo2Max => DayPatch::Vo2Max(
            serde_json::from_value::<raw::Vo2Day>(item.clone())
                .map_err(json)?
                .into(),
        ),
    })
}

/// Parse one item of the activity search response.
pub fn parse_activity(item: &Value) -> Result<Activity, ParseError> {
    let a: raw::Activity =
        serde_json::from_value(item.clone()).map_err(|err| ParseError::Json {
            what: "activity",
            err,
        })?;
    let start_local = a
        .start_time_local
        .as_deref()
        .and_then(|s| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok())
        .ok_or(ParseError::ActivityStart)?;
    Ok(Activity {
        id: a.activity_id,
        start_local,
        name: a.activity_name,
        type_key: a
            .activity_type
            .and_then(|t| t.type_key)
            .unwrap_or_else(|| "unknown".to_owned()),
        distance_m: a.distance,
        duration_s: a.duration,
        moving_s: a.moving_duration,
        avg_hr: a.average_hr,
        max_hr: a.max_hr,
        avg_speed_mps: a.average_speed,
        calories: a.calories,
        elevation_gain_m: a.elevation_gain,
        aerobic_te: a.aerobic_training_effect,
        training_load: a.activity_training_load,
    })
}

/// Serde mirrors of the Garmin JSON. Unknown fields are ignored. Field names follow
/// Garmin's JSON, hence the naming-lint allowances.
#[allow(clippy::struct_field_names)]
mod raw {
    use super::{DailySummary, Deserialize, HrvSummary, SleepSummary, Vo2Max};

    #[derive(Debug, Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    pub struct Daily {
        calories: Calories,
        heart_rate: HeartRate,
        movement: Movement,
        stress: Stress,
        body_battery: BodyBattery,
        intensity_minutes: Intensity,
    }
    #[derive(Debug, Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    struct Calories {
        burned_active: Option<f64>,
        burned_resting: Option<f64>,
        burned_total: Option<f64>,
    }
    #[derive(Debug, Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    struct HeartRate {
        resting_value: Option<i64>,
        min_value: Option<i64>,
        max_value: Option<i64>,
    }
    #[derive(Debug, Deserialize, Default)]
    #[serde(default)]
    struct Movement {
        steps: Steps,
    }
    #[derive(Debug, Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    struct Steps {
        value: Option<i64>,
        goal: Option<i64>,
        distance_in_meters: Option<f64>,
    }
    #[derive(Debug, Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    struct Stress {
        avg_level: Option<i64>,
    }
    #[derive(Debug, Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    struct BodyBattery {
        max_value: Option<i64>,
        min_value: Option<i64>,
    }
    #[derive(Debug, Deserialize, Default)]
    #[serde(default)]
    struct Intensity {
        moderate: Option<i64>,
        vigorous: Option<i64>,
    }

    impl From<Daily> for DailySummary {
        fn from(d: Daily) -> Self {
            Self {
                steps: d.movement.steps.value,
                step_goal: d.movement.steps.goal,
                distance_m: d.movement.steps.distance_in_meters,
                active_kcal: d.calories.burned_active,
                resting_kcal: d.calories.burned_resting,
                total_kcal: d.calories.burned_total,
                resting_hr: d.heart_rate.resting_value,
                min_hr: d.heart_rate.min_value,
                max_hr: d.heart_rate.max_value,
                // Garmin reports -1/-2 when stress could not be measured.
                stress_avg: d.stress.avg_level.filter(|v| *v >= 0),
                body_battery_max: d.body_battery.max_value,
                body_battery_min: d.body_battery.min_value,
                intensity_moderate: d.intensity_minutes.moderate,
                intensity_vigorous: d.intensity_minutes.vigorous,
            }
        }
    }

    #[derive(Debug, Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    pub struct Sleep {
        sleep_scores: SleepScores,
        sleep_time_seconds: Option<i64>,
        deep_sleep_seconds: Option<i64>,
        light_sleep_seconds: Option<i64>,
        rem_sleep_seconds: Option<i64>,
        awake_sleep_seconds: Option<i64>,
    }
    #[derive(Debug, Deserialize, Default)]
    #[serde(default)]
    struct SleepScores {
        overall: Option<Score>,
    }
    #[derive(Debug, Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    struct Score {
        value: Option<i64>,
        qualifier_key: Option<String>,
    }

    impl From<Sleep> for SleepSummary {
        fn from(s: Sleep) -> Self {
            let overall = s.sleep_scores.overall.unwrap_or_default();
            Self {
                score: overall.value,
                qualifier: overall.qualifier_key,
                sleep_seconds: s.sleep_time_seconds.filter(|v| *v > 0),
                deep_seconds: s.deep_sleep_seconds,
                light_seconds: s.light_sleep_seconds,
                rem_seconds: s.rem_sleep_seconds,
                awake_seconds: s.awake_sleep_seconds,
            }
        }
    }

    #[derive(Debug, Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    pub struct Hrv {
        last_night_avg: Option<i64>,
        weekly_avg: Option<i64>,
        status: Option<String>,
        baseline: Option<Baseline>,
    }
    #[derive(Debug, Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    struct Baseline {
        balanced_low: Option<i64>,
        balanced_upper: Option<i64>,
    }

    impl From<Hrv> for HrvSummary {
        fn from(h: Hrv) -> Self {
            let base = h.baseline.unwrap_or_default();
            Self {
                last_night_avg: h.last_night_avg,
                weekly_avg: h.weekly_avg,
                status: h.status,
                baseline_low: base.balanced_low,
                baseline_high: base.balanced_upper,
            }
        }
    }

    #[derive(Debug, Deserialize, Default)]
    #[serde(default)]
    pub struct Vo2Day {
        generic: Option<Vo2Entry>,
        cycling: Option<Vo2Entry>,
    }
    #[derive(Debug, Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    struct Vo2Entry {
        vo2_max_precise_value: Option<f64>,
        vo2_max_value: Option<f64>,
    }
    impl Vo2Entry {
        fn value(&self) -> Option<f64> {
            self.vo2_max_precise_value.or(self.vo2_max_value)
        }
    }

    impl From<Vo2Day> for Vo2Max {
        fn from(v: Vo2Day) -> Self {
            Self {
                running: v.generic.as_ref().and_then(Vo2Entry::value),
                cycling: v.cycling.as_ref().and_then(Vo2Entry::value),
            }
        }
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Activity {
        pub activity_id: i64,
        #[serde(default)]
        pub activity_name: Option<String>,
        #[serde(default)]
        pub activity_type: Option<ActivityType>,
        #[serde(default)]
        pub start_time_local: Option<String>,
        #[serde(default)]
        pub distance: Option<f64>,
        #[serde(default)]
        pub duration: Option<f64>,
        #[serde(default)]
        pub moving_duration: Option<f64>,
        #[serde(default, rename = "averageHR")]
        pub average_hr: Option<f64>,
        #[serde(default, rename = "maxHR")]
        pub max_hr: Option<f64>,
        #[serde(default)]
        pub average_speed: Option<f64>,
        #[serde(default)]
        pub calories: Option<f64>,
        #[serde(default)]
        pub elevation_gain: Option<f64>,
        #[serde(default)]
        pub aerobic_training_effect: Option<f64>,
        #[serde(default)]
        pub activity_training_load: Option<f64>,
    }
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ActivityType {
        #[serde(default)]
        pub type_key: Option<String>,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let path = format!(
            "{}/../../tests/fixtures/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
    }

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn daily_summary() {
        let days = split_days(Source::Daily, &fixture("dailysummaries")).unwrap();
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].0, d("2026-01-10"));
        let DayPatch::Daily(s) = parse_day(Source::Daily, &days[0].1).unwrap() else {
            panic!("wrong patch")
        };
        assert_eq!(s.steps, Some(9_120));
        assert_eq!(s.step_goal, Some(8_000));
        assert_eq!(s.resting_hr, Some(52));
        assert_eq!(s.active_kcal, Some(650.0));
        assert_eq!(s.total_kcal, Some(2_950.0));
        assert_eq!(s.stress_avg, Some(31));
        assert_eq!(s.body_battery_max, Some(88));
        // second day: unmeasured stress (-1) and missing blocks
        let DayPatch::Daily(s) = parse_day(Source::Daily, &days[1].1).unwrap() else {
            panic!("wrong patch")
        };
        assert_eq!(s.stress_avg, None);
        assert_eq!(s.resting_hr, None);
    }

    #[test]
    fn sleep_summary() {
        let days = split_days(Source::Sleep, &fixture("sleep")).unwrap();
        assert_eq!(days.len(), 2);
        let DayPatch::Sleep(s) = parse_day(Source::Sleep, &days[0].1).unwrap() else {
            panic!("wrong patch")
        };
        assert_eq!(s.score, Some(82));
        assert_eq!(s.qualifier.as_deref(), Some("GOOD"));
        assert_eq!(s.sleep_seconds, Some(27_000));
        let DayPatch::Sleep(s) = parse_day(Source::Sleep, &days[1].1).unwrap() else {
            panic!("wrong patch")
        };
        assert_eq!(s.score, None);
    }

    #[test]
    fn hrv_summary() {
        let days = split_days(Source::Hrv, &fixture("hrv")).unwrap();
        assert_eq!(days.len(), 1);
        let DayPatch::Hrv(h) = parse_day(Source::Hrv, &days[0].1).unwrap() else {
            panic!("wrong patch")
        };
        assert_eq!(h.last_night_avg, Some(61));
        assert_eq!(h.weekly_avg, Some(58));
        assert_eq!(h.status.as_deref(), Some("BALANCED"));
        assert_eq!((h.baseline_low, h.baseline_high), (Some(50), Some(72)));
    }

    #[test]
    fn vo2max() {
        let days = split_days(Source::Vo2Max, &fixture("vo2max")).unwrap();
        assert_eq!(days.len(), 2);
        assert_eq!(days[1].0, d("2026-01-11"));
        let DayPatch::Vo2Max(v) = parse_day(Source::Vo2Max, &days[0].1).unwrap() else {
            panic!("wrong patch")
        };
        assert_eq!(v.running, Some(48.5));
        assert_eq!(v.cycling, None);
        let DayPatch::Vo2Max(v) = parse_day(Source::Vo2Max, &days[1].1).unwrap() else {
            panic!("wrong patch")
        };
        assert_eq!(v.cycling, Some(45.0));
    }

    #[test]
    fn activities() {
        let body = fixture("activities");
        let items = body.as_array().unwrap();
        let run = parse_activity(&items[0]).unwrap();
        assert!(run.is_run());
        assert_eq!(run.id, 1001);
        assert_eq!(run.avg_hr, Some(150.0));
        let pace = run.pace_s_per_km().unwrap();
        assert!((pace - 300.0).abs() < 0.01, "pace was {pace}");
        let walk = parse_activity(&items[1]).unwrap();
        assert!(!walk.is_run());
        assert_eq!(walk.training_load, None);
    }

    #[test]
    fn empty_and_bad_shapes() {
        assert!(split_days(Source::Sleep, &Value::Null).unwrap().is_empty());
        assert!(
            split_days(Source::Daily, &serde_json::json!({}))
                .unwrap()
                .is_empty()
        );
        assert!(split_days(Source::Sleep, &serde_json::json!("x")).is_err());
        assert!(split_days(Source::Sleep, &serde_json::json!([{"nope": 1}])).is_err());
    }
}
