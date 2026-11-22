//! Rendering. Pure function of [`App`] state.

use chrono::{Days, Local, NaiveDate};
use gartop_core::model::{Activity, DailyMetrics};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Style, Stylize as _};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Axis, Bar, BarChart, BarGroup, Block, BorderType, Cell, Chart, Clear, Dataset, GraphType,
    LegendPosition, Paragraph, Row, Table,
};

use crate::app::{App, Range, Tab};
use crate::fetch::{FetchOutcome, FetchState};
use crate::stats::{self, Metric};

const C_RHR: Color = Color::LightRed;
const C_HRV: Color = Color::LightMagenta;
const C_SLEEP: Color = Color::LightBlue;
const C_STEPS: Color = Color::LightGreen;
const C_KCAL: Color = Color::Yellow;
const C_VO2: Color = Color::LightCyan;
const C_BATTERY: Color = Color::Cyan;
const C_STRESS: Color = Color::LightYellow;
const C_RUN: Color = Color::LightGreen;
const C_DIM: Color = Color::DarkGray;
const C_GOOD: Color = Color::Green;
const C_BAD: Color = Color::Red;

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

// ------------------------------------------------------------------ metrics

fn m_rhr(d: &DailyMetrics) -> Option<f64> {
    d.resting_hr.map(|v| v as f64)
}
fn m_hrv(d: &DailyMetrics) -> Option<f64> {
    d.hrv_last_night.map(|v| v as f64)
}
fn m_hrv_week(d: &DailyMetrics) -> Option<f64> {
    d.hrv_weekly_avg.map(|v| v as f64)
}
fn m_hrv_low(d: &DailyMetrics) -> Option<f64> {
    d.hrv_baseline_low.map(|v| v as f64)
}
fn m_hrv_high(d: &DailyMetrics) -> Option<f64> {
    d.hrv_baseline_high.map(|v| v as f64)
}
fn m_sleep(d: &DailyMetrics) -> Option<f64> {
    d.sleep_score.map(|v| v as f64)
}
fn m_sleep_h(d: &DailyMetrics) -> Option<f64> {
    d.sleep_seconds.map(|v| v as f64 / 3600.0)
}
fn m_deep_h(d: &DailyMetrics) -> Option<f64> {
    d.deep_sleep_seconds.map(|v| v as f64 / 3600.0)
}
fn m_rem_h(d: &DailyMetrics) -> Option<f64> {
    d.rem_sleep_seconds.map(|v| v as f64 / 3600.0)
}
fn m_steps(d: &DailyMetrics) -> Option<f64> {
    d.steps.map(|v| v as f64)
}
fn m_step_goal(d: &DailyMetrics) -> Option<f64> {
    d.step_goal.map(|v| v as f64)
}
fn m_kcal(d: &DailyMetrics) -> Option<f64> {
    d.total_kcal
}
fn m_active_kcal(d: &DailyMetrics) -> Option<f64> {
    d.active_kcal
}
fn m_vo2(d: &DailyMetrics) -> Option<f64> {
    d.vo2max_running
}
fn m_bb_max(d: &DailyMetrics) -> Option<f64> {
    d.body_battery_max.map(|v| v as f64)
}
fn m_bb_min(d: &DailyMetrics) -> Option<f64> {
    d.body_battery_min.map(|v| v as f64)
}
fn m_stress(d: &DailyMetrics) -> Option<f64> {
    d.stress_avg.map(|v| v as f64)
}

// --------------------------------------------------------------- formatting

fn fmt_int(v: f64) -> String {
    let n = v.round() as i64;
    let s = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

fn fmt_1(v: f64) -> String {
    format!("{v:.1}")
}

fn fmt_0(v: f64) -> String {
    format!("{v:.0}")
}

fn fmt_k(v: f64) -> String {
    if v.abs() >= 1000.0 {
        format!("{:.1}k", v / 1000.0)
    } else {
        fmt_0(v)
    }
}

fn fmt_pace_min(v: f64) -> String {
    stats::fmt_min_sec(v * 60.0)
}

fn fmt_hours_f(v: f64) -> String {
    stats::fmt_hours(v * 3600.0)
}

fn title_case(s: &str) -> String {
    let lower = s.to_lowercase().replace('_', " ");
    let mut c = lower.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

fn status_color(status: &str) -> Color {
    match status {
        "BALANCED" | "EXCELLENT" | "GOOD" => C_GOOD,
        "UNBALANCED" | "FAIR" => Color::Yellow,
        "LOW" | "POOR" => C_BAD,
        _ => Color::Gray,
    }
}

fn panel(title: &str, color: Color) -> Block<'_> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(C_DIM))
        .title(Span::styled(
            format!(" {title} "),
            Style::new().fg(color).bold(),
        ))
}

// -------------------------------------------------------------------- frame

pub fn draw(f: &mut Frame, app: &App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(f.area());
    draw_header(f, header, app);
    if app.data.days.is_empty() && app.data.activities.is_empty() {
        draw_empty(f, body);
    } else {
        match app.tab {
            Tab::Overview => draw_overview(f, body, app),
            Tab::Running => draw_running(f, body, app),
            Tab::Recovery => draw_recovery(f, body, app),
        }
    }
    draw_footer(f, footer, app);
    if app.show_help {
        draw_help(f, f.area());
    }
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![Span::styled(
        " gartop ",
        Style::new().fg(Color::Black).bg(C_VO2).bold(),
    )];
    spans.push(Span::raw(" "));
    for (i, tab) in Tab::ALL.iter().enumerate() {
        let label = format!(" {} {} ", i + 1, tab.title());
        spans.push(if *tab == app.tab {
            Span::styled(label, Style::new().fg(Color::Black).bg(Color::White).bold())
        } else {
            Span::styled(label, Style::new().fg(Color::Gray))
        });
    }
    spans.push(Span::styled("  │  ", Style::new().fg(C_DIM)));
    for r in Range::ALL {
        let label = format!(" {}:{} ", r.key(), r.label());
        spans.push(if r == app.range {
            Span::styled(label, Style::new().fg(Color::Black).bg(C_KCAL).bold())
        } else {
            Span::styled(label, Style::new().fg(Color::Gray))
        });
    }
    f.render_widget(Line::from(spans), area);

    let synced = app.data.last_sync.map_or_else(
        || "never synced".to_owned(),
        |t| {
            format!(
                "synced {}",
                t.with_timezone(&Local).format("%a %d %b %H:%M")
            )
        },
    );
    f.render_widget(
        Line::from(Span::styled(
            format!("{synced} "),
            Style::new().fg(Color::Gray),
        ))
        .alignment(Alignment::Right),
        area,
    );
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let key = |k: &'static str| Span::styled(k, Style::new().fg(C_VO2).bold());
    let txt = |t: &'static str| Span::styled(t, Style::new().fg(Color::Gray));
    let hints = Line::from(vec![
        txt(" "),
        key("q"),
        txt(" quit  "),
        key("1-3/Tab"),
        txt(" page  "),
        key("w m t y"),
        txt(" range  "),
        key("f"),
        txt(" fetch now  "),
        key("r"),
        txt(" reload  "),
        key("?"),
        txt(" help"),
    ]);
    f.render_widget(hints, area);

    let status = match (&app.error, &app.fetch) {
        (_, FetchState::Running { started, .. }) => Span::styled(
            format!(
                "{} fetching… {}s ",
                SPINNER[(app.tick % SPINNER.len() as u64) as usize],
                started.elapsed().as_secs()
            ),
            Style::new().fg(C_KCAL).bold(),
        ),
        (Some(err), _) => Span::styled(format!("db error: {err} "), Style::new().fg(C_BAD)),
        (None, FetchState::Done { at, outcome }) => match outcome {
            FetchOutcome::Ok => Span::styled(
                format!("✔ updated {} ", at.format("%H:%M")),
                Style::new().fg(C_GOOD),
            ),
            FetchOutcome::AlreadyRunning => Span::styled(
                "⏳ fetch already running elsewhere ".to_owned(),
                Style::new().fg(C_KCAL),
            ),
            FetchOutcome::Failed(msg) => {
                Span::styled(format!("✘ fetch failed: {msg} "), Style::new().fg(C_BAD))
            }
        },
        (None, FetchState::Idle) => Span::raw(""),
    };
    f.render_widget(Line::from(status).alignment(Alignment::Right), area);
}

fn draw_empty(f: &mut Frame, area: Rect) {
    let text = vec![
        Line::from("No data yet.".bold()),
        Line::from(""),
        Line::from(vec![
            "Press ".into(),
            "f".fg(C_VO2).bold(),
            " to fetch the last two days, or run".into(),
        ]),
        Line::from("gartop-fetch backfill --since YYYY-MM-DD".fg(C_KCAL)),
        Line::from("to import your history."),
    ];
    let h = text.len() as u16;
    let [_, mid, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(h),
        Constraint::Fill(1),
    ])
    .areas(area);
    f.render_widget(Paragraph::new(text).alignment(Alignment::Center), mid);
}

fn draw_help(f: &mut Frame, area: Rect) {
    let rows = [
        ("1 2 3, Tab, ←/→", "switch page"),
        ("w m t y", "range: 7 days, 30 days, 90 days, 1 year"),
        ("+ / -, ↑/↓", "widen / narrow range"),
        ("f", "fetch the latest data now (gartop-fetch sync)"),
        ("r", "reload the database"),
        ("?", "toggle this help"),
        ("q, Esc", "quit"),
    ];
    let mut lines: Vec<Line> = rows
        .iter()
        .map(|(k, v)| {
            Line::from(vec![
                Span::styled(format!("{k:>16}  "), Style::new().fg(C_VO2).bold()),
                Span::raw(*v),
            ])
        })
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(
        "The view also reloads by itself when the fetcher writes new data.".fg(Color::Gray),
    ));
    let w = 72.min(area.width);
    let h = (lines.len() as u16 + 2).min(area.height);
    let popup = Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    );
    f.render_widget(Clear, popup);
    f.render_widget(Paragraph::new(lines).block(panel("Help", C_VO2)), popup);
}

// ------------------------------------------------------------------- tiles

#[derive(Clone, Copy)]
enum Better {
    Higher,
    Lower,
    Neither,
}

/// Formats a metric value for display.
type Fmt = fn(f64) -> String;

struct Tile {
    label: &'static str,
    color: Color,
    value: String,
    delta: Option<(f64, Better, Fmt)>,
    note: Line<'static>,
}

fn tile_for(
    app: &App,
    label: &'static str,
    color: Color,
    f: Metric,
    better: Better,
    fmt: Fmt,
    unit: &str,
) -> (Tile, Option<NaiveDate>) {
    let days = &app.data.days;
    match stats::latest(days, app.today, f) {
        Some((day, v)) => (
            Tile {
                label,
                color,
                value: format!("{}{unit}", fmt(v)),
                delta: stats::delta_vs_week(days, day, v, f).map(|d| (d, better, fmt)),
                note: Line::from(""),
            },
            Some(day),
        ),
        None => (
            Tile {
                label,
                color,
                value: "n/a".into(),
                delta: None,
                note: Line::from(""),
            },
            None,
        ),
    }
}

fn age_note(today: NaiveDate, day: Option<NaiveDate>) -> Option<Span<'static>> {
    let day = day?;
    let ago = (today - day).num_days();
    (ago > 0).then(|| {
        Span::styled(
            if ago == 1 {
                "yesterday".to_owned()
            } else {
                format!("{ago}d ago")
            },
            Style::new().fg(C_DIM),
        )
    })
}

fn render_tiles(f: &mut Frame, area: Rect, tiles: Vec<Tile>) {
    let areas = Layout::horizontal(vec![Constraint::Fill(1); tiles.len()]).split(area);
    for (tile, area) in tiles.into_iter().zip(areas.iter()) {
        let delta = match tile.delta {
            Some((d, better, fmt)) if fmt(d.abs()) != fmt(0.0) => {
                let up = d > 0.0;
                let color = match (better, up) {
                    (Better::Higher, true) | (Better::Lower, false) => C_GOOD,
                    (Better::Higher, false) | (Better::Lower, true) => C_BAD,
                    (Better::Neither, _) => Color::Gray,
                };
                Line::from(vec![
                    Span::styled(
                        format!("{} {}", if up { "▲" } else { "▼" }, fmt(d.abs())),
                        Style::new().fg(color),
                    ),
                    Span::styled(" vs 7d", Style::new().fg(C_DIM)),
                ])
            }
            Some(_) => Line::from(Span::styled("= 7d avg", Style::new().fg(C_DIM))),
            None => Line::from(""),
        };
        let lines = vec![
            Line::from(Span::styled(tile.value, Style::new().fg(tile.color).bold())),
            delta,
            tile.note,
        ];
        f.render_widget(
            Paragraph::new(lines).block(panel(tile.label, tile.color)),
            *area,
        );
    }
}

// ------------------------------------------------------------------- charts

struct Series {
    name: &'static str,
    color: Color,
    points: Vec<(f64, f64)>,
    kind: GraphType,
}

impl Series {
    fn line(name: &'static str, color: Color, points: Vec<(f64, f64)>) -> Self {
        Self {
            name,
            color,
            points,
            kind: GraphType::Line,
        }
    }
    fn bar(name: &'static str, color: Color, points: Vec<(f64, f64)>) -> Self {
        Self {
            name,
            color,
            points,
            kind: GraphType::Bar,
        }
    }
    fn dots(name: &'static str, color: Color, points: Vec<(f64, f64)>) -> Self {
        Self {
            name,
            color,
            points,
            kind: GraphType::Scatter,
        }
    }
}

struct ChartSpec<'a> {
    title: &'a str,
    color: Color,
    from: NaiveDate,
    span_days: u64,
    zero_based: bool,
    y_fmt: Fmt,
}

fn render_chart(f: &mut Frame, area: Rect, spec: &ChartSpec<'_>, series: &[Series]) {
    let block = panel(spec.title, spec.color);
    let all = series.iter().flat_map(|s| s.points.iter());
    let (mut lo, mut hi) = all.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), (_, y)| {
        (lo.min(*y), hi.max(*y))
    });
    if !lo.is_finite() {
        let msg = Paragraph::new("no data in range".fg(C_DIM))
            .alignment(Alignment::Center)
            .block(block);
        f.render_widget(msg, area);
        return;
    }
    if spec.zero_based {
        lo = lo.min(0.0);
    }
    let pad = ((hi - lo) * 0.08).max(if spec.zero_based { 0.0 } else { 1.0 });
    hi += pad;
    if !spec.zero_based {
        lo -= pad;
    }
    if (hi - lo).abs() < f64::EPSILON {
        hi = lo + 1.0;
    }

    let x_max = (spec.span_days.max(2) - 1) as f64;
    let date_at = |x: u64| {
        spec.from
            .checked_add_days(Days::new(x))
            .unwrap_or(spec.from)
    };
    let date_fmt = if spec.span_days > 120 {
        "%b %y"
    } else {
        "%b %d"
    };
    let x_labels = [
        0,
        spec.span_days.saturating_sub(1) / 2,
        spec.span_days.saturating_sub(1),
    ]
    .map(|x| {
        Span::styled(
            date_at(x).format(date_fmt).to_string(),
            Style::new().fg(C_DIM),
        )
    });
    let y_labels =
        [lo, lo.midpoint(hi), hi].map(|y| Span::styled((spec.y_fmt)(y), Style::new().fg(C_DIM)));

    let datasets: Vec<Dataset> = series
        .iter()
        .map(|s| {
            let ds = Dataset::default()
                .marker(if matches!(s.kind, GraphType::Scatter) {
                    Marker::Dot
                } else {
                    Marker::Braille
                })
                .graph_type(s.kind)
                .style(Style::new().fg(s.color))
                .data(&s.points);
            if s.name.is_empty() {
                ds
            } else {
                ds.name(s.name)
            }
        })
        .collect();
    let chart = Chart::new(datasets)
        .block(block)
        .x_axis(
            Axis::default()
                .bounds([0.0, x_max])
                .labels(x_labels)
                .style(Style::new().fg(C_DIM)),
        )
        .y_axis(
            Axis::default()
                .bounds([lo, hi])
                .labels(y_labels)
                .style(Style::new().fg(C_DIM)),
        )
        .legend_position(Some(LegendPosition::TopLeft))
        .hidden_legend_constraints((Constraint::Ratio(1, 1), Constraint::Ratio(1, 1)));
    f.render_widget(chart, area);
}

// ----------------------------------------------------------------- overview

fn draw_overview(f: &mut Frame, area: Rect, app: &App) {
    let [tiles, row1, row2] = Layout::vertical([
        Constraint::Length(5),
        Constraint::Fill(1),
        Constraint::Fill(1),
    ])
    .areas(area);
    render_tiles(f, tiles, overview_tiles(app));
    draw_overview_charts(f, row1, row2, app);
}

fn overview_tiles(app: &App) -> Vec<Tile> {
    let days = &app.data.days;
    let latest_day = |day: Option<NaiveDate>| day.and_then(|d| days.iter().find(|m| m.day == d));

    let (mut rhr, rhr_day) = tile_for(
        app,
        "Resting HR",
        C_RHR,
        m_rhr,
        Better::Lower,
        fmt_0,
        " bpm",
    );
    rhr.note = Line::from(age_note(app.today, rhr_day).unwrap_or_default());

    let (mut hrv, hrv_day) = tile_for(app, "HRV", C_HRV, m_hrv, Better::Higher, fmt_0, " ms");
    if let Some(m) = latest_day(hrv_day) {
        let status = m.hrv_status.clone().unwrap_or_default();
        hrv.note = Line::from(vec![
            Span::styled(title_case(&status), Style::new().fg(status_color(&status))),
            Span::styled(
                m.hrv_weekly_avg
                    .map(|w| format!(" · 7d {w}"))
                    .unwrap_or_default(),
                Style::new().fg(C_DIM),
            ),
        ]);
    }

    let (mut sleep, sleep_day) = tile_for(
        app,
        "Sleep score",
        C_SLEEP,
        m_sleep,
        Better::Higher,
        fmt_0,
        "",
    );
    if let Some(m) = latest_day(sleep_day) {
        let q = m.sleep_qualifier.clone().unwrap_or_default();
        sleep.note = Line::from(vec![
            Span::styled(title_case(&q), Style::new().fg(status_color(&q))),
            Span::styled(
                m.sleep_seconds
                    .map(|s| format!(" · {}", stats::fmt_hours(s as f64)))
                    .unwrap_or_default(),
                Style::new().fg(C_DIM),
            ),
        ]);
    }

    let (mut steps, steps_day) =
        tile_for(app, "Steps", C_STEPS, m_steps, Better::Higher, fmt_int, "");
    if let Some(m) = latest_day(steps_day) {
        steps.note = Line::from(Span::styled(
            m.step_goal
                .map(|g| format!("goal {}", fmt_int(g as f64)))
                .unwrap_or_default(),
            Style::new().fg(C_DIM),
        ));
    }

    let (mut kcal, kcal_day) = tile_for(
        app,
        "Calories",
        C_KCAL,
        m_kcal,
        Better::Neither,
        fmt_int,
        " kcal",
    );
    if let Some(m) = latest_day(kcal_day) {
        kcal.note = Line::from(Span::styled(
            m.active_kcal
                .map(|a| format!("active {}", fmt_int(a)))
                .unwrap_or_default(),
            Style::new().fg(C_DIM),
        ));
    }

    let (mut vo2, vo2_day) = tile_for(app, "VO2 max", C_VO2, m_vo2, Better::Higher, fmt_1, "");
    vo2.delta = None; // reported sporadically; a 7-day delta is mostly noise
    vo2.note = Line::from(
        age_note(app.today, vo2_day)
            .unwrap_or_else(|| Span::styled("running", Style::new().fg(C_DIM))),
    );

    let (mut bb, bb_day) = tile_for(
        app,
        "Body battery",
        C_BATTERY,
        m_bb_max,
        Better::Higher,
        fmt_0,
        "",
    );
    if let Some(m) = latest_day(bb_day) {
        bb.note = Line::from(Span::styled(
            m.body_battery_min
                .map(|v| format!("low {v}"))
                .unwrap_or_default(),
            Style::new().fg(C_DIM),
        ));
    }

    vec![rhr, hrv, sleep, steps, kcal, vo2, bb]
}

fn draw_overview_charts(f: &mut Frame, row1: Rect, row2: Rect, app: &App) {
    let days = &app.data.days;
    let from = app.from();
    let to = app.today;
    let span = app.range.days();
    let spec = |title, color, zero_based, y_fmt| ChartSpec {
        title,
        color,
        from,
        span_days: span,
        zero_based,
        y_fmt,
    };

    let [a, b, c] = Layout::horizontal([Constraint::Fill(1); 3]).areas(row1);
    render_chart(
        f,
        a,
        &spec("Resting heart rate", C_RHR, false, fmt_0),
        &[Series::line(
            "",
            C_RHR,
            stats::series(days, from, to, m_rhr),
        )],
    );
    render_chart(
        f,
        b,
        &spec("HRV (overnight)", C_HRV, false, fmt_0),
        &[
            Series::line("nightly", C_HRV, stats::series(days, from, to, m_hrv)),
            Series::line(
                "7d avg",
                Color::Gray,
                stats::series(days, from, to, m_hrv_week),
            ),
        ],
    );
    render_chart(
        f,
        c,
        &spec("Sleep score", C_SLEEP, false, fmt_0),
        &[Series::line(
            "",
            C_SLEEP,
            stats::series(days, from, to, m_sleep),
        )],
    );

    let [a, b, c] = Layout::horizontal([Constraint::Fill(1); 3]).areas(row2);
    render_chart(
        f,
        a,
        &spec("Steps", C_STEPS, true, fmt_k),
        &[
            Series::bar("", C_STEPS, stats::series(days, from, to, m_steps)),
            Series::line("goal", C_DIM, stats::series(days, from, to, m_step_goal)),
        ],
    );
    render_chart(
        f,
        b,
        &spec("Calories burned", C_KCAL, true, fmt_k),
        &[
            Series::bar("total", C_KCAL, stats::series(days, from, to, m_kcal)),
            Series::bar(
                "active",
                Color::LightRed,
                stats::series(days, from, to, m_active_kcal),
            ),
        ],
    );
    render_chart(
        f,
        c,
        &spec("VO2 max (running)", C_VO2, false, fmt_1),
        &[Series::line(
            "",
            C_VO2,
            stats::carried_series(days, from, to, m_vo2),
        )],
    );
}

// ------------------------------------------------------------------ running

fn draw_running(f: &mut Frame, area: Rect, app: &App) {
    let [tiles, charts, table] = Layout::vertical([
        Constraint::Length(5),
        Constraint::Fill(1),
        Constraint::Fill(1),
    ])
    .areas(area);
    let from = app.from();
    let runs: Vec<&Activity> = app
        .data
        .activities
        .iter()
        .filter(|a| a.is_run() && a.start_local.date() >= from && a.start_local.date() <= app.today)
        .collect();
    let s = stats::summarize_runs(runs.iter().copied());

    let plain = |label, color, value: String, note: String| Tile {
        label,
        color,
        value,
        delta: None,
        note: Line::from(Span::styled(note, Style::new().fg(C_DIM))),
    };
    let range = format!("last {}", app.range.label());
    render_tiles(
        f,
        tiles,
        vec![
            plain("Runs", C_RUN, s.count.to_string(), range.clone()),
            plain("Distance", C_RUN, format!("{:.1} km", s.km), range),
            plain(
                "Moving time",
                C_VO2,
                stats::fmt_duration(s.moving_s),
                String::new(),
            ),
            plain(
                "Avg pace",
                C_KCAL,
                s.pace_s_per_km()
                    .map_or("n/a".into(), |p| format!("{}/km", stats::fmt_min_sec(p))),
                String::new(),
            ),
            plain(
                "Longest",
                C_SLEEP,
                format!("{:.1} km", s.longest_km),
                String::new(),
            ),
            plain(
                "Avg HR",
                C_RHR,
                s.avg_hr.map_or("n/a".into(), |h| format!("{h:.0} bpm")),
                String::new(),
            ),
        ],
    );

    let [weekly, pace] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)]).areas(charts);
    draw_weekly_km(f, weekly, app);

    let points: Vec<(f64, f64)> = runs
        .iter()
        .filter_map(|a| {
            Some((
                (a.start_local.date() - from).num_days() as f64,
                a.pace_s_per_km()? / 60.0,
            ))
        })
        .collect();
    let spec = ChartSpec {
        title: "Pace per run (min/km)",
        color: C_KCAL,
        from,
        span_days: app.range.days(),
        zero_based: false,
        y_fmt: fmt_pace_min,
    };
    render_chart(
        f,
        pace,
        &spec,
        &[
            Series::line("", C_DIM, points.clone()),
            Series::dots("", C_KCAL, points),
        ],
    );

    draw_runs_table(f, table, &runs);
}

fn draw_weekly_km(f: &mut Frame, area: Rect, app: &App) {
    let inner_w = area.width.saturating_sub(2);
    let bar_w: u16 = 5;
    let fit = (inner_w / (bar_w + 1)).max(1) as usize;
    let wanted = ((app.range.days() / 7) as usize).clamp(8, 52);
    let weeks = stats::weekly_run_km(&app.data.activities, app.today, wanted.min(fit));
    let bars: Vec<Bar> = weeks
        .iter()
        .map(|(start, km)| {
            Bar::default()
                .value((km * 10.0).round() as u64)
                .text_value(if *km > 0.0 {
                    format!("{km:.0}")
                } else {
                    String::new()
                })
                .label(Line::from(start.format("%d/%m").to_string()))
                .style(Style::new().fg(C_RUN))
                .value_style(Style::new().fg(Color::Black).bg(C_RUN))
        })
        .collect();
    let chart = BarChart::default()
        .block(panel("Weekly running distance (km)", C_RUN))
        .data(BarGroup::default().bars(&bars))
        .bar_width(bar_w)
        .bar_gap(1)
        .label_style(Style::new().fg(C_DIM));
    f.render_widget(chart, area);
}

fn draw_runs_table(f: &mut Frame, area: Rect, runs: &[&Activity]) {
    let header = Row::new([
        "Date", "Name", "Type", "Dist", "Time", "Pace", "Avg HR", "TE", "Load",
    ])
    .style(Style::new().fg(C_VO2).bold());
    let rows: Vec<Row> = runs
        .iter()
        .rev()
        .map(|a| {
            let opt = |v: Option<f64>, f: fn(f64) -> String| v.map_or_else(|| "–".to_owned(), f);
            Row::new(vec![
                Cell::from(a.start_local.format("%a %d %b").to_string()),
                Cell::from(a.name.clone().unwrap_or_default()),
                Cell::from(title_case(
                    &a.type_key
                        .replace("_running", "")
                        .replace("running", "road"),
                )),
                Cell::from(opt(a.distance_m, |m| format!("{:.2} km", m / 1000.0))),
                Cell::from(opt(a.moving_s.or(a.duration_s), stats::fmt_duration)),
                Cell::from(opt(a.pace_s_per_km(), |p| {
                    format!("{}/km", stats::fmt_min_sec(p))
                })),
                Cell::from(opt(a.avg_hr, fmt_0)),
                Cell::from(opt(a.aerobic_te, fmt_1)),
                Cell::from(opt(a.training_load, fmt_0)),
            ])
        })
        .collect();
    let empty = rows.is_empty();
    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Fill(1),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(6),
            Constraint::Length(4),
            Constraint::Length(5),
        ],
    )
    .header(header)
    .column_spacing(2)
    .block(panel("Runs", C_RUN));
    f.render_widget(table, area);
    if empty {
        let inner = area.inner(Margin::new(1, 2));
        f.render_widget(
            Paragraph::new("no runs in range".fg(C_DIM)).alignment(Alignment::Center),
            inner,
        );
    }
}

// ----------------------------------------------------------------- recovery

fn draw_recovery(f: &mut Frame, area: Rect, app: &App) {
    let [row1, row2, table] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Fill(1),
        Constraint::Length(10),
    ])
    .areas(area);
    let days = &app.data.days;
    let from = app.from();
    let to = app.today;
    let span = app.range.days();
    let spec = |title, color, zero_based, y_fmt| ChartSpec {
        title,
        color,
        from,
        span_days: span,
        zero_based,
        y_fmt,
    };

    let [a, b] = Layout::horizontal([Constraint::Fill(1); 2]).areas(row1);
    render_chart(
        f,
        a,
        &spec("HRV vs baseline (ms)", C_HRV, false, fmt_0),
        &[
            Series::line("baseline", C_DIM, stats::series(days, from, to, m_hrv_low)),
            Series::line("", C_DIM, stats::series(days, from, to, m_hrv_high)),
            Series::line(
                "7d avg",
                Color::Gray,
                stats::series(days, from, to, m_hrv_week),
            ),
            Series::line("nightly", C_HRV, stats::series(days, from, to, m_hrv)),
        ],
    );
    render_chart(
        f,
        b,
        &spec("Sleep (hours)", C_SLEEP, true, fmt_hours_f),
        &[
            Series::bar("total", C_SLEEP, stats::series(days, from, to, m_sleep_h)),
            Series::line("deep", Color::Blue, stats::series(days, from, to, m_deep_h)),
            Series::line("REM", C_HRV, stats::series(days, from, to, m_rem_h)),
        ],
    );

    let [a, b] = Layout::horizontal([Constraint::Fill(1); 2]).areas(row2);
    render_chart(
        f,
        a,
        &spec("Body battery", C_BATTERY, true, fmt_0),
        &[
            Series::line("high", C_BATTERY, stats::series(days, from, to, m_bb_max)),
            Series::line("low", Color::Blue, stats::series(days, from, to, m_bb_min)),
        ],
    );
    render_chart(
        f,
        b,
        &spec("Average stress", C_STRESS, true, fmt_0),
        &[Series::line(
            "",
            C_STRESS,
            stats::series(days, from, to, m_stress),
        )],
    );

    draw_nights_table(f, table, app);
}

fn draw_nights_table(f: &mut Frame, area: Rect, app: &App) {
    let header = Row::new([
        "Night", "Score", "Quality", "Asleep", "Deep", "Light", "REM", "Awake", "HRV", "Status",
        "RHR",
    ])
    .style(Style::new().fg(C_VO2).bold());
    let hours = |v: Option<i64>| v.map_or_else(|| "–".to_owned(), |s| stats::fmt_hours(s as f64));
    let num = |v: Option<i64>| v.map_or_else(|| "–".to_owned(), |n| n.to_string());
    let visible = area.height.saturating_sub(3) as usize;
    let rows: Vec<Row> = app
        .data
        .days
        .iter()
        .rev()
        .filter(|d| d.day <= app.today && (d.sleep_score.is_some() || d.sleep_seconds.is_some()))
        .take(visible)
        .map(|d| {
            let q = d.sleep_qualifier.clone().unwrap_or_default();
            let st = d.hrv_status.clone().unwrap_or_default();
            Row::new(vec![
                Cell::from(d.day.format("%a %d %b").to_string()),
                Cell::from(num(d.sleep_score)),
                Cell::from(Span::styled(
                    title_case(&q),
                    Style::new().fg(status_color(&q)),
                )),
                Cell::from(hours(d.sleep_seconds)),
                Cell::from(hours(d.deep_sleep_seconds)),
                Cell::from(hours(d.light_sleep_seconds)),
                Cell::from(hours(d.rem_sleep_seconds)),
                Cell::from(hours(d.awake_seconds)),
                Cell::from(num(d.hrv_last_night)),
                Cell::from(Span::styled(
                    title_case(&st),
                    Style::new().fg(status_color(&st)),
                )),
                Cell::from(num(d.resting_hr)),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(5),
            Constraint::Length(9),
            Constraint::Length(7),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(4),
            Constraint::Length(10),
            Constraint::Length(4),
        ],
    )
    .header(header)
    .column_spacing(2)
    .block(panel("Recent nights", C_SLEEP));
    f.render_widget(table, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Data;
    use chrono::NaiveDateTime;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 3, 31).unwrap()
    }

    /// Deterministic synthetic data: 120 days of metrics and a run every third day.
    fn sample() -> Data {
        let mut days = Vec::new();
        let mut activities = Vec::new();
        for i in 0..120_i64 {
            let day = today() - Days::new(119 - i as u64);
            let wave = ((i % 7) - 3) as f64;
            days.push(DailyMetrics {
                day,
                steps: Some(7_000 + i * 20 + (i % 5) * 900),
                step_goal: Some(8_000),
                active_kcal: Some(500.0 + wave * 40.0),
                resting_kcal: Some(2_000.0),
                total_kcal: Some(2_500.0 + wave * 40.0),
                resting_hr: Some(58 - i / 30 + (i % 3)),
                stress_avg: Some(30 + (i % 9)),
                body_battery_max: Some(70 + (i % 6) * 4),
                body_battery_min: Some(15 + (i % 4) * 3),
                sleep_score: Some(70 + (i % 11)),
                sleep_qualifier: Some(if i % 11 > 7 { "GOOD" } else { "FAIR" }.into()),
                sleep_seconds: Some(25_000 + (i % 5) * 900),
                deep_sleep_seconds: Some(4_800),
                light_sleep_seconds: Some(14_000),
                rem_sleep_seconds: Some(5_400),
                awake_seconds: Some(600),
                hrv_last_night: Some(50 + (i % 8)),
                hrv_weekly_avg: Some(53),
                hrv_status: Some("BALANCED".into()),
                hrv_baseline_low: Some(48),
                hrv_baseline_high: Some(66),
                vo2max_running: (i % 10 == 0).then_some(44.0 + i as f64 / 60.0),
                ..DailyMetrics::default()
            });
            if i % 3 == 0 {
                activities.push(Activity {
                    id: i,
                    start_local: NaiveDateTime::new(
                        day,
                        chrono::NaiveTime::from_hms_opt(7, 30, 0).unwrap(),
                    ),
                    name: Some(format!("Run {i}")),
                    type_key: if i % 2 == 0 {
                        "running"
                    } else {
                        "treadmill_running"
                    }
                    .into(),
                    distance_m: Some(5_000.0 + (i % 4) as f64 * 1_500.0),
                    duration_s: None,
                    moving_s: Some(1_800.0 + (i % 4) as f64 * 520.0),
                    avg_hr: Some(148.0 + (i % 5) as f64),
                    max_hr: Some(175.0),
                    avg_speed_mps: None,
                    calories: Some(450.0),
                    elevation_gain_m: Some(40.0),
                    aerobic_te: Some(3.1),
                    training_load: Some(90.0),
                });
            }
        }
        Data {
            days,
            activities,
            last_sync: None,
        }
    }

    fn render(app: &App) -> TestBackend {
        let mut term = Terminal::new(TestBackend::new(150, 42)).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
        term.backend().clone()
    }

    #[test]
    fn overview_snapshot() {
        let app = App::new(sample(), today());
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn running_snapshot() {
        let mut app = App::new(sample(), today());
        app.tab = Tab::Running;
        app.range = Range::Quarter;
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn recovery_snapshot() {
        let mut app = App::new(sample(), today());
        app.tab = Tab::Recovery;
        app.range = Range::Week;
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn empty_and_help_snapshot() {
        let mut app = App::new(Data::default(), today());
        app.show_help = true;
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn tiny_terminal_does_not_panic() {
        for (w, h) in [(1, 1), (20, 5), (40, 12)] {
            let app = App::new(sample(), today());
            for tab in Tab::ALL {
                let mut app = App {
                    tab,
                    ..App::new(Data::default(), today())
                };
                app.data = sample();
                let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                term.draw(|f| draw(f, &app)).unwrap();
            }
            drop(app);
        }
    }

    #[test]
    fn int_formatting() {
        assert_eq!(fmt_int(1_234_567.0), "1,234,567");
        assert_eq!(fmt_int(999.0), "999");
        assert_eq!(fmt_int(-1_000.0), "-1,000");
        assert_eq!(fmt_k(12_340.0), "12.3k");
        assert_eq!(title_case("treadmill_running"), "Treadmill running");
    }
}
