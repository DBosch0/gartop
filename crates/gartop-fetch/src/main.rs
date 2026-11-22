//! `gartop-fetch`: pulls Garmin Connect data into the local gartop database.
//!
//! Exit codes: 0 success, 1 error, 2 usage error, 3 another fetch is already running,
//! 4 the sync finished but some requests failed.

mod auth;
mod client;
mod sync;

use std::fs::File;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context as _;
use chrono::{Days, Local, NaiveDate, Utc};
use clap::{Parser, Subcommand};
use gartop_core::{Config, Paths, Store, config};

use crate::auth::TokenProvider;
use crate::client::{GarminClient, Http};
use crate::sync::SyncOptions;

const EXIT_LOCKED: u8 = 3;
const EXIT_PARTIAL: u8 = 4;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Increase log verbosity (-v debug, -vv trace).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Re-fetch the most recent days (default command).
    Sync {
        /// Number of days back from today to fetch (default from config, 7).
        #[arg(short, long)]
        days: Option<u32>,
        /// Keep running, syncing again after each interval (e.g. "4h", "30m").
        #[arg(short, long, value_parser = humantime::parse_duration)]
        watch: Option<Duration>,
    },
    /// Fetch history from a start date, skipping ranges already stored.
    Backfill {
        /// First day to fetch (YYYY-MM-DD).
        #[arg(long)]
        since: NaiveDate,
        /// Re-fetch ranges that are already stored.
        #[arg(long)]
        force: bool,
    },
    /// Rebuild the derived tables from the stored raw JSON (no network).
    Reparse,
    /// Print the raw JSON returned by an API path.
    Probe {
        /// Path relative to the API base, e.g. /hrv-service/hrv/2026-11-20
        path: String,
        /// Query parameters as key=value.
        #[arg(short, long = "query", value_parser = parse_kv)]
        query: Vec<(String, String)>,
    },
}

fn parse_kv(s: &str) -> Result<(String, String), String> {
    s.split_once('=')
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .ok_or_else(|| format!("expected key=value, got {s:?}"))
}

/// Outcome of a command that maps onto a process exit code.
enum Outcome {
    Ok,
    Locked,
    Partial,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging(cli.verbose);
    match run(cli) {
        Ok(Outcome::Ok) => ExitCode::SUCCESS,
        Ok(Outcome::Locked) => {
            eprintln!("gartop-fetch: another fetch is already running");
            ExitCode::from(EXIT_LOCKED)
        }
        Ok(Outcome::Partial) => ExitCode::from(EXIT_PARTIAL),
        Err(e) => {
            eprintln!("gartop-fetch: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<Outcome> {
    let paths = Paths::discover()?;
    let cfg = Config::load(&paths)?;
    let db = cfg.db_path(&paths);
    let command = cli.command.unwrap_or(Command::Sync {
        days: None,
        watch: None,
    });

    match command {
        Command::Sync { days, watch } => {
            let days = days.unwrap_or(cfg.sync_days).max(1);
            let Some(interval) = watch else {
                return sync_once(&paths, &cfg, &db, days);
            };
            tracing::info!(
                "watch mode: syncing every {}",
                humantime::format_duration(interval)
            );
            loop {
                match sync_once(&paths, &cfg, &db, days) {
                    Ok(Outcome::Locked) => tracing::warn!("skipped: another fetch is running"),
                    Ok(_) => {}
                    Err(e) => tracing::error!("sync failed: {e:#}"),
                }
                std::thread::sleep(interval);
            }
        }
        Command::Backfill { since, force } => {
            let Some(_lock) = try_lock(&paths.lock_file())? else {
                return Ok(Outcome::Locked);
            };
            let mut client = make_client(&paths, &cfg)?;
            let mut store = Store::open(&db)?;
            let opts = SyncOptions {
                from: since,
                to: Local::now().date_naive(),
                skip_existing: !force,
            };
            let report = sync::run(&mut client, &mut store, opts);
            finish(&store, &report)
        }
        Command::Reparse => {
            let Some(_lock) = try_lock(&paths.lock_file())? else {
                return Ok(Outcome::Locked);
            };
            let n = Store::open(&db)?.reparse()?;
            tracing::info!("reparsed {n} stored items");
            Ok(Outcome::Ok)
        }
        Command::Probe { path, query } => {
            let mut client = make_client(&paths, &cfg)?;
            let query: Vec<(&str, String)> =
                query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
            let value = client
                .get_json(&path, &query)
                .with_context(|| format!("probing {path}"))?;
            println!("{}", serde_json::to_string_pretty(&value)?);
            Ok(Outcome::Ok)
        }
    }
}

fn sync_once(paths: &Paths, cfg: &Config, db: &Path, days: u32) -> anyhow::Result<Outcome> {
    let Some(_lock) = try_lock(&paths.lock_file())? else {
        return Ok(Outcome::Locked);
    };
    let mut client = make_client(paths, cfg)?;
    let mut store = Store::open(db)?;
    let to = Local::now().date_naive();
    let from = to
        .checked_sub_days(Days::new(u64::from(days - 1)))
        .unwrap_or(to);
    let report = sync::run(
        &mut client,
        &mut store,
        SyncOptions {
            from,
            to,
            skip_existing: false,
        },
    );
    finish(&store, &report)
}

fn finish(store: &Store, report: &sync::SyncReport) -> anyhow::Result<Outcome> {
    tracing::info!(
        "stored {} day items and {} activities ({} chunks already present)",
        report.days_stored,
        report.activities_stored,
        report.chunks_skipped
    );
    if report.failures.is_empty() {
        store.set_last_sync(Utc::now())?;
        Ok(Outcome::Ok)
    } else {
        for f in &report.failures {
            eprintln!("failed: {f}");
        }
        Ok(Outcome::Partial)
    }
}

/// Take the exclusive fetch lock, or `None` if another process holds it.
/// The lock is released when the returned file is dropped.
fn try_lock(path: &Path) -> anyhow::Result<Option<File>> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .with_context(|| format!("opening lock file {}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => {
            Err(e).with_context(|| format!("locking {}", path.display()))
        }
    }
}

fn make_client(paths: &Paths, cfg: &Config) -> anyhow::Result<GarminClient> {
    let key = config::api_key(paths)?;
    let http = Http::new(&cfg.base_url, Duration::from_millis(300));
    Ok(GarminClient::new(
        http,
        TokenProvider::new(key, paths.token_file()),
    ))
}

fn init_logging(verbose: u8) {
    let default = match verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    let filter = tracing_subscriber::EnvFilter::try_from_env("GARTOP_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_is_exclusive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fetch.lock");
        let first = try_lock(&path).unwrap();
        assert!(first.is_some());
        assert!(try_lock(&path).unwrap().is_none());
        drop(first);
        assert!(try_lock(&path).unwrap().is_some());
    }

    #[test]
    fn cli_parses() {
        use clap::CommandFactory as _;
        Cli::command().debug_assert();
        let cli =
            Cli::try_parse_from(["gartop-fetch", "sync", "--days", "2", "--watch", "4h"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Sync { days: Some(2), watch: Some(w) }) if w == Duration::from_secs(4 * 3600)
        ));
    }
}
