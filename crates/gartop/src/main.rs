//! `gartop`: an htop/btop-style terminal dashboard for Garmin data stored by `gartop-fetch`.

mod app;
mod fetch;
mod stats;
mod ui;

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context as _;
use chrono::Local;
use clap::Parser;
use gartop_core::{Config, Paths, Store};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event};

use crate::app::{Action, App, Data};

/// Redraw interval; also how often the spinner advances and the DB is checked for changes.
const TICK: Duration = Duration::from_millis(250);

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Database file (default: from config, or ~/.local/share/gartop/gartop.db).
    #[arg(long)]
    db: Option<PathBuf>,
    /// Program used by the `f` key (default: gartop-fetch next to this binary, else on PATH).
    #[arg(long)]
    fetch_cmd: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let paths = Paths::discover()?;
    let cfg = Config::load(&paths)?;
    let db = cli.db.unwrap_or_else(|| cfg.db_path(&paths));
    let fetcher = fetch::fetcher_program(cli.fetch_cmd);

    // Fail before touching the terminal if the database can't be opened.
    let store = Store::open(&db).with_context(|| format!("opening {}", db.display()))?;
    let today = Local::now().date_naive();
    let app = App::new(Data::load(&store, today)?, today);

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, app, &store, &fetcher);
    ratatui::restore();
    result
}

fn run(
    terminal: &mut DefaultTerminal,
    mut app: App,
    store: &Store,
    fetcher: &Path,
) -> anyhow::Result<()> {
    // One long-lived connection: `data_version` changes whenever the fetcher commits.
    let mut seen = store.data_version().ok();
    loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        let mut reload = false;
        if event::poll(TICK)? {
            match event::read()? {
                Event::Key(key) => match app.handle_key(key) {
                    Action::Quit => return Ok(()),
                    Action::Reload => reload = true,
                    Action::Fetch => app.fetch = fetch::spawn(fetcher.to_path_buf()),
                    Action::None => {}
                },
                Event::Resize(..) => {}
                _ => continue,
            }
        } else {
            app.tick = app.tick.wrapping_add(1);
        }

        reload |= fetch::poll(&mut app.fetch);
        let version = store.data_version().ok();
        if version != seen {
            seen = version;
            reload = true;
        }
        if reload {
            app.today = Local::now().date_naive();
            match Data::load(store, app.today) {
                Ok(data) => {
                    app.data = data;
                    app.error = None;
                }
                Err(e) => app.error = Some(format!("{e:#}")),
            }
        }
    }
}
