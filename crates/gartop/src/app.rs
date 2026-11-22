//! Viewer state and key handling (no I/O and no rendering).

use chrono::{DateTime, Days, NaiveDate, Utc};
use gartop_core::Store;
use gartop_core::model::{Activity, DailyMetrics};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::fetch::FetchState;

/// Dashboard pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Running,
    Recovery,
}

impl Tab {
    pub const ALL: [Self; 3] = [Self::Overview, Self::Running, Self::Recovery];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Running => "Running",
            Self::Recovery => "Recovery",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    fn cycle(self, forward: bool) -> Self {
        let n = Self::ALL.len();
        let i = self.index();
        Self::ALL[if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        }]
    }
}

/// Time window shown by the charts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    Week,
    Month,
    Quarter,
    Year,
}

impl Range {
    pub const ALL: [Self; 4] = [Self::Week, Self::Month, Self::Quarter, Self::Year];

    pub const fn days(self) -> u64 {
        match self {
            Self::Week => 7,
            Self::Month => 30,
            Self::Quarter => 90,
            Self::Year => 365,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Week => "7d",
            Self::Month => "30d",
            Self::Quarter => "90d",
            Self::Year => "1y",
        }
    }

    pub const fn key(self) -> char {
        match self {
            Self::Week => 'w',
            Self::Month => 'm',
            Self::Quarter => 't',
            Self::Year => 'y',
        }
    }
}

/// Everything loaded from the database.
#[derive(Debug, Default)]
pub struct Data {
    pub days: Vec<DailyMetrics>,
    pub activities: Vec<Activity>,
    pub last_sync: Option<DateTime<Utc>>,
}

impl Data {
    /// Load the longest range plus a week of lead-in (for "vs 7d" comparisons).
    pub fn load(store: &Store, today: NaiveDate) -> anyhow::Result<Self> {
        let from = today
            .checked_sub_days(Days::new(Range::Year.days() + 7))
            .unwrap_or(today);
        Ok(Self {
            days: store.daily_range(from, today)?,
            activities: store.activities(from, today)?,
            last_sync: store.last_sync()?,
        })
    }
}

/// What the event loop should do after a key press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    Reload,
    Fetch,
}

/// Viewer state.
#[derive(Debug)]
pub struct App {
    pub tab: Tab,
    pub range: Range,
    pub data: Data,
    pub today: NaiveDate,
    pub show_help: bool,
    pub fetch: FetchState,
    /// Last database error, shown in the status bar.
    pub error: Option<String>,
    /// Frame counter, drives the spinner.
    pub tick: u64,
}

impl App {
    pub fn new(data: Data, today: NaiveDate) -> Self {
        Self {
            tab: Tab::Overview,
            range: Range::Month,
            data,
            today,
            show_help: false,
            fetch: FetchState::Idle,
            error: None,
            tick: 0,
        }
    }

    /// First day of the selected range.
    pub fn from(&self) -> NaiveDate {
        self.today
            .checked_sub_days(Days::new(self.range.days() - 1))
            .unwrap_or(self.today)
    }

    pub fn is_fetching(&self) -> bool {
        matches!(self.fetch, FetchState::Running { .. })
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.kind != KeyEventKind::Press {
            return Action::None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }
        if self.show_help {
            // Any key closes the help; q still quits.
            self.show_help = false;
            return if key.code == KeyCode::Char('q') {
                Action::Quit
            } else {
                Action::None
            };
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Action::Quit,
            KeyCode::Char('?' | 'h') => self.show_help = true,
            KeyCode::Char('r') => return Action::Reload,
            KeyCode::Char('f') if !self.is_fetching() => return Action::Fetch,
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => self.tab = self.tab.cycle(true),
            KeyCode::BackTab | KeyCode::Left => self.tab = self.tab.cycle(false),
            KeyCode::Char(c @ '1'..='3') => {
                self.tab = Tab::ALL[(c as usize) - ('1' as usize)];
            }
            KeyCode::Char('+') | KeyCode::Up => self.range = self.step_range(1),
            KeyCode::Char('-') | KeyCode::Down => self.range = self.step_range(-1),
            KeyCode::Char(c) => {
                if let Some(r) = Range::ALL.into_iter().find(|r| r.key() == c) {
                    self.range = r;
                }
            }
            _ => {}
        }
        Action::None
    }

    fn step_range(&self, dir: isize) -> Range {
        let i = Range::ALL
            .iter()
            .position(|r| *r == self.range)
            .unwrap_or(1) as isize;
        Range::ALL[(i + dir).clamp(0, Range::ALL.len() as isize - 1) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(app: &mut App, code: KeyCode) -> Action {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn app() -> App {
        App::new(
            Data::default(),
            NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
        )
    }

    #[test]
    fn keys_switch_tabs_and_ranges() {
        let mut a = app();
        press(&mut a, KeyCode::Char('2'));
        assert_eq!(a.tab, Tab::Running);
        press(&mut a, KeyCode::Tab);
        assert_eq!(a.tab, Tab::Recovery);
        press(&mut a, KeyCode::Tab);
        assert_eq!(a.tab, Tab::Overview);
        press(&mut a, KeyCode::BackTab);
        assert_eq!(a.tab, Tab::Recovery);

        press(&mut a, KeyCode::Char('y'));
        assert_eq!(a.range, Range::Year);
        assert_eq!(a.from(), NaiveDate::from_ymd_opt(2025, 4, 1).unwrap());
        press(&mut a, KeyCode::Char('+'));
        assert_eq!(a.range, Range::Year, "clamped at the widest range");
        press(&mut a, KeyCode::Char('w'));
        assert_eq!(a.from(), NaiveDate::from_ymd_opt(2026, 3, 25).unwrap());
    }

    #[test]
    fn actions() {
        let mut a = app();
        assert_eq!(press(&mut a, KeyCode::Char('f')), Action::Fetch);
        assert_eq!(press(&mut a, KeyCode::Char('r')), Action::Reload);
        a.fetch = crate::fetch::FetchState::Running {
            started: std::time::Instant::now(),
            rx: std::sync::mpsc::channel().1,
        };
        assert_eq!(
            press(&mut a, KeyCode::Char('f')),
            Action::None,
            "no double fetch"
        );
        assert_eq!(press(&mut a, KeyCode::Char('?')), Action::None);
        assert!(a.show_help);
        assert_eq!(press(&mut a, KeyCode::Char('x')), Action::None);
        assert!(!a.show_help);
        assert_eq!(press(&mut a, KeyCode::Char('q')), Action::Quit);
    }
}
