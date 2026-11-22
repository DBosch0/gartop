# gartop

An htop/btop-style terminal dashboard for your Garmin watch. It shows trends in resting heart
rate, HRV, sleep, steps, calories, VO2 max, body battery, stress and running.

```text
 gartop   1 Overview  2 Running  3 Recovery   │   w:7d  m:30d  t:90d  y:1y
╭ Resting HR ───────╮╭ HRV ───────────────╮╭ Sleep score ──────╮╭ Steps ─────────────╮╭ Calories ─
│57 bpm             ││57 ms               ││79                 ││12,980              ││2,380 kcal
│▲ 1 vs 7d          ││▲ 4 vs 7d           ││▲ 4 vs 7d          ││▲ 1,751 vs 7d       ││▼ 120 vs 7d
│                   ││Balanced · 7d 53    ││Good · 7h57m       ││goal 8,000          ││active 380
╰───────────────────╯╰────────────────────╯╰───────────────────╯╰────────────────────╯╰───────────
╭ Resting heart rate ────────────────────────────╮╭ HRV (overnight) ──────────────────────────────
│58   │                                          ││58   │┌───────┐
│     │                                          ││     ││nightly│         ⢠           ⡄
│     │                                          ││     ││7d avg⡇│        ⢠⢻          ⡜⡇         ⢠
│     │  ⢰    ⡆   ⡆   ⢰    ⡆   ⡆   ⢰   ⢰    ⡆   ⢰││     │└───────┘       ⢠⠃⠘⡄        ⡜ ⡇        ⢠⠃
│     │  ⡟⡄  ⢸⢣  ⢰⢣   ⡟⡄  ⢸⡇  ⢸⢣   ⡟⡄  ⡟⡄  ⢸⢣   ⡇││     │    ⡜  ⢸       ⢠⠃  ⡇       ⡜  ⢱       ⢠⠃
│     │ ⢰⠁⡇  ⡎⢸  ⢸⢸  ⢰⠁⡇  ⡎⢱  ⡎⢸  ⢰⠁⡇ ⢰⠁⡇  ⡎⢸  ⢰⠁││     │   ⡜   ⢸      ⢠⠃   ⡇      ⡜   ⢸      ⢠⠃
│     │ ⡎ ⡇ ⢰⠁⢸  ⡇⢸  ⡎ ⡇ ⢰⠁⢸ ⢰⠁⢸  ⡎ ⡇ ⡎ ⡇ ⢰⠁⢸  ⡎ ││     │  ⡜    ⢸     ⢠⠃    ⡇     ⡜    ⢸     ⢠⠃
│56   │⢀⠇ ⢸ ⡜  ⡇⢠⠃ ⡇⢠⠃ ⢸ ⡜ ⢸ ⡜  ⡇⢠⠃ ⢸⢀⠇ ⢸ ⡜  ⡇⢠⠃ ││54   │⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤
│     │⢸  ⢸⢀⠇  ⡇⡸  ⡇⡸  ⢸⢀⠇ ⠸⣀⠇  ⡇⡸  ⢸⢸  ⢸⢀⠇  ⡇⡸  ││     │⡸       ⡇  ⢠⠃      ⢸   ⡜      ⢸   ⢠⠃
│     │⡜  ⠈⡾   ⢱⠇  ⢱⠇  ⠈⡾   ⡿   ⢱⠇  ⠈⡞  ⠈⡾   ⢱⠇  ││     │⠁       ⡇ ⢠⠃       ⢸  ⡜       ⠘⡄ ⢠⠃
│     │⠃   ⠃   ⠘   ⠘    ⠃   ⠃   ⠘    ⠃   ⠃   ⠘   ││     │        ⢣ ⡎        ⠘⡄⡜         ⡇⢠⠃
```

<sub>Excerpt of the Overview page, rendered from synthetic test data.</sub>

## How it works

gartop is two programs that share one local SQLite database:

```text
 Garmin Personal API ──► gartop-fetch ──► ~/.local/share/gartop/gartop.db ──► gartop
                       (cron / service)                                    (dashboard)
                              ▲                                               │
                              └───────────── `f` key: fetch now ──────────────┘
```

| binary         | what it does                                                              |
|----------------|---------------------------------------------------------------------------|
| `gartop-fetch` | Talks to the [Garmin Connect Personal API] and stores the responses. Run it on a schedule. |
| `gartop`       | The dashboard. It reads only the local database, so it starts instantly, works offline and keeps your full history. |

Because the history is stored locally, trends cover everything you have ever fetched, not just
what the API returns at one time.

[Garmin Connect Personal API]: https://developer.garmin.com/gc-developer-program/personal/

## What it shows

| page         | contents                                                                           |
|--------------|------------------------------------------------------------------------------------|
| **Overview** | tiles comparing today with your 7-day average, plus charts for resting HR, overnight HRV, sleep score, steps (with your goal), calories (total and active) and VO2 max |
| **Running**  | run count, distance, moving time, average pace, longest run, average HR, weekly distance bars, pace trend and a table of runs (with training effect and load) |
| **Recovery** | HRV against your baseline band, sleep duration and stages, body battery, stress, and a table of recent nights |

Every chart can show 7 days, 30 days, 90 days or 1 year.

## Requirements

- Rust 1.89 or newer
- A Garmin Connect **personal API key**. You create it in your Garmin Connect account; see Garmin's [personal API docs][Garmin Connect Personal API].
- A terminal with Unicode and 256-colour support. The charts use Braille characters.

## Installation

```sh
git clone <this repo> && cd gartop
cargo install --path crates/gartop-fetch --root ~/.local
cargo install --path crates/gartop       --root ~/.local
```

Both binaries go in `~/.local/bin`; make sure that directory is on your `PATH`. The dashboard
runs the `gartop-fetch` that sits next to its own binary.

### API key

The fetcher checks these places in order:

1. the `GARMIN_API_KEY` environment variable
2. `./.env` in the current directory
3. `~/.config/gartop/.env`, which is the one to use for cron and services

```sh
install -Dm600 /dev/stdin ~/.config/gartop/.env <<< 'GARMIN_API_KEY=your-key-here'
```

The key is exchanged for a short-lived access token (about 24 hours). The token is cached in
`~/.cache/gartop/token` with mode 0600 and renewed automatically.

### First run

```sh
gartop-fetch backfill --since 2024-01-01   # one-off history import; a few seconds per year
gartop
```

## Usage

### gartop (dashboard)

| key                         | action                                                 |
|-----------------------------|--------------------------------------------------------|
| `1` `2` `3`, `Tab`, `←` `→` | Overview / Running / Recovery page                     |
| `w` `m` `t` `y`             | range: 7 days, 30 days, 90 days, 1 year                |
| `+` `-`, `↑` `↓`            | widen or narrow the range                              |
| `f`                         | fetch the latest data now (`gartop-fetch sync --days 2`) |
| `r`                         | reload                                                 |
| `?`                         | help                                                   |
| `q`, `Esc`                  | quit                                                   |

- **Auto-reload:** the dashboard reloads by itself whenever the fetcher writes new data, including runs started by cron or a service.
- **Status bar:** shows fetch progress and the result.
- **Options:**
  - `--db PATH` opens a different database.
  - `--fetch-cmd PROGRAM` sets the program the `f` key runs.

### gartop-fetch (fetcher)

```text
gartop-fetch [sync] [--days N] [--watch 4h]    re-fetch the last N days (default 7)
gartop-fetch backfill --since DATE [--force]    import history, skipping ranges already stored
gartop-fetch reparse                            rebuild the derived tables from the stored raw JSON
gartop-fetch probe PATH [-q key=value]...       print the raw JSON of any API endpoint
```

- **Why re-fetch 7 days:** `sync` re-fetches the past 7 days because Garmin revises recent sleep and HRV data afterwards.
- **`--watch`:** keeps the process running and syncs after each interval. Use it with service supervisors.
- **Lock:** only one fetch runs at a time. A second one exits with code 3.
- **Logs:** go to stderr. Set the level with `-v`/`-vv` or `GARTOP_LOG=debug`.

| exit code | meaning                                     |
|-----------|---------------------------------------------|
| 0         | success                                     |
| 1         | error                                       |
| 2         | invalid command-line arguments              |
| 3         | another fetch is already running            |
| 4         | the sync finished, but some requests failed |

### Scheduling

`contrib/` has ready-made files. Edit the username or paths in the one you pick:

| file                                    | init system | how                                               |
|-----------------------------------------|-------------|---------------------------------------------------|
| `contrib/crontab`                       | cron        | `crontab -e`, then paste the line (every 4 hours)  |
| `contrib/openrc/gartop-fetch` + `.confd` | OpenRC     | `/etc/init.d/` + `/etc/conf.d/`, then `rc-update add gartop-fetch` |
| `contrib/runit/gartop-fetch/`           | runit       | copy to `/etc/runit/sv/` and link into the service directory |
| `contrib/dinit/gartop-fetch`            | dinit       | user service in `~/.config/dinit.d/`               |
| `contrib/systemd/gartop-fetch.{service,timer}` | systemd | `~/.config/systemd/user/`, then `systemctl --user enable --now gartop-fetch.timer` |

## Configuration

`~/.config/gartop/config.toml` is optional. All keys are shown with their defaults:

```toml
sync_days = 7                                        # days re-fetched by `gartop-fetch sync`
base_url = "https://personal.connectapi.garmin.com"
# db_path = "/somewhere/else/gartop.db"
```

| path                               | contents                     |
|------------------------------------|------------------------------|
| `~/.config/gartop/`                | `config.toml`, `.env`        |
| `~/.local/share/gartop/gartop.db`  | the database (SQLite, WAL)   |
| `~/.local/share/gartop/fetch.lock` | fetch lock                   |
| `~/.cache/gartop/token`            | cached access token          |

## Data model

| table           | contents                                                                         |
|-----------------|----------------------------------------------------------------------------------|
| `raw_daily`     | The source of truth: the raw API JSON for each day, from the `daily`, `sleep`, `hrv` and `vo2max` sources. |
| `daily_metrics` | One typed row per day, derived from `raw_daily`.                                  |
| `activities`    | One row per activity, with its raw JSON.                                          |
| `meta`          | The time of the last successful sync.                                             |

`gartop-fetch reparse` rebuilds the derived data from the raw JSON without contacting Garmin.
So new metrics can be computed from history you already have.

You can query the database directly:

```sh
sqlite3 ~/.local/share/gartop/gartop.db \
  "SELECT day, resting_hr, hrv_last_night, sleep_score, steps FROM daily_metrics ORDER BY day DESC LIMIT 7"
```

## Troubleshooting

| symptom                                | fix                                                                          |
|----------------------------------------|------------------------------------------------------------------------------|
| `GARMIN_API_KEY not found`             | Put the key in `~/.config/gartop/.env`. A service or cron job doesn't read your shell's environment. |
| `HTTP 401` from the token endpoint     | The API key expired or was deleted. Create a new one in Garmin Connect.      |
| `another fetch is already running`     | A cron or service sync holds the lock. Wait for it to finish; the dashboard reloads by itself. |
| Dashboard says "No data yet"           | Run `gartop-fetch backfill --since YYYY-MM-DD`, or press `f`.                 |
| `f` says `could not run gartop-fetch`  | Install `gartop-fetch` next to `gartop` or on your `PATH`, or pass `--fetch-cmd`. |
| A metric looks wrong                   | Run `gartop-fetch probe <path>` to see the raw JSON, then `gartop-fetch reparse` after fixing the parser. |

## Roadmap

- **Training readiness and training status:** the Personal API doesn't offer these (the endpoints return 404). The plan is to estimate them from the training load, HRV, sleep and resting heart rate history that is already stored.
- **Body weight and strength-training progress:** may come from other data sources.

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo insta review        # after intentional UI changes (snapshot tests)
```

| path                   | contents                                                                |
|------------------------|-------------------------------------------------------------------------|
| `crates/gartop-core/`  | config and paths, models, lenient JSON parsers, SQLite store + migrations |
| `crates/gartop-fetch/` | token handling, HTTP client, sync and backfill, CLI                     |
| `crates/gartop/`       | ratatui dashboard: state (`app.rs`), rendering (`ui.rs`), stats, background fetch |
| `tests/fixtures/`      | **synthetic** API responses (the real structure, no personal data)      |
| `contrib/`             | scheduler and service files                                             |
