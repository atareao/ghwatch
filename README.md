# ghwatch

[Español](README.es.md) · **English**

[![CI](https://github.com/atareao/ghwatch/actions/workflows/ci.yml/badge.svg)](https://github.com/atareao/ghwatch/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/atareao/ghwatch/blob/main/LICENSE)

`ghwatch` is a Rust CLI that watches every GitHub repository of a single account
for vulnerabilities, so you no longer have to open them one by one.

It collects **Dependabot alerts**, **secret scanning alerts** and **code scanning
alerts**; enriches them with [OSV.dev](https://osv.dev) (CVSS score, CVE alias
and the version that fixes the issue); keeps a **SQLite history** between runs;
produces a **console, Markdown and JSON report** that is always written to disk;
and optionally **notifies [Valet](https://github.com/atareao/valet)** through its
ingest API, in a *best-effort* way.

## Installation

Requires a recent toolchain for **Rust edition 2024** (Rust 1.85 or newer) and a
GitHub token with the classic **`repo`** scope.

```bash
# From a local checkout (installs into ~/.local/bin, where the systemd unit
# expects it)
cargo install --path . --locked --root "$HOME/.local"

# Straight from git
cargo install --git https://github.com/atareao/ghwatch --locked --root "$HOME/.local"
```

Without `--root`, `cargo install` drops the binary in `~/.cargo/bin/ghwatch`; if
you prefer that, adjust `ExecStart` in `ghwatch.service` accordingly. You can also
run it from the source tree with `cargo run --release -- <subcommand>`.

## Quick start

```bash
# 1. Create the database and the configuration template (~600 permissions)
ghwatch init

# 2. Add your GitHub token (and, optionally, the Valet ingest key)
$EDITOR ~/.config/ghwatch/config.toml

# 3. Scan the account, persist the run and write the report
ghwatch scan
```

By default the database lives at `~/.local/share/ghwatch/ghwatch.db` and the
reports at `~/.local/share/ghwatch/reports/ghwatch-YYYY-MM-DD.md` and
`...-YYYY-MM-DD.json` (one pair per day). The scan ends with a one-line summary;
use `--quiet` to silence the progress bar and the final summary.

## Subcommands

| Command | What it does |
|---|---|
| `ghwatch scan` | Scans every repository, persists the run, writes the report and optionally notifies Valet. |
| `ghwatch report` | Re-emits the latest local report. **No network.** |
| `ghwatch diff` | Shows the alerts that were new and resolved in the latest run. **No network.** |
| `ghwatch history` | Shows the stored run series (date, total, new, resolved). **No network.** |
| `ghwatch repos` | Lists every repository with open alerts, its alert count and its severity breakdown. **No network.** |
| `ghwatch notify` | Resends the latest local report to Valet without rewriting it. |
| `ghwatch init` | Creates the database and the config template, without overwriting existing ones. |

```bash
# Scan and include forks/archived repositories
ghwatch scan --include-forks --include-archived

# Scan without OSV enrichment (faster, no third-party calls)
ghwatch scan --no-enrich

# Re-emit the latest report as JSON on stdout (stderr keeps the logs)
ghwatch report --format json > report.json

# Changes since the last run, and the last 10 runs
ghwatch diff
ghwatch history --limit 10

# Repositories with open alerts, biggest first
ghwatch repos --limit 5

# Send the latest report again, even without changes
ghwatch notify --force
```

`report`, `diff`, `history` and `repos` **never touch GitHub or Valet**: they read
only the local database and the report files, so you can rerun them offline. Only
`scan` scans, and only `scan`/`notify` can reach Valet.

`scan` flags: `--include-forks`, `--include-archived`, `--no-enrich`, `--force`,
`--quiet`, `-v/--verbose` (repeatable), `--db`, `--report-dir`, `--config`,
`--concurrency`, `--github-base-url`, `--osv-base-url`, `--valet-base-url`.
Run `ghwatch <subcommand> --help` for the full list.

`repos` reads only the local database (it neither creates it nor touches the
network) and prints to **stdout**:

- `--limit <N>`: caps how many repositories are shown; default `20`, and `0`
  shows all.
- `--all`: shows all of them (same as `--limit 0`); if both are given, `--all`
  wins.
- `--min-severity <critical|high|medium|low|unknown>`: keeps only the
  repositories with at least one alert of that severity **or higher**, without
  trimming the count or the breakdown shown (the repository is listed whole);
  `unknown` lets every repository with alerts through.
- `--format <console|json|markdown>`: output format, `console` by default, always
  on stdout (no file is written).

```
$ ghwatch repos --limit 5
ejecución #3 · generado 2026-11-07T08:25:50.008680277+00:00 · repositorios con alertas: 88 de 363 · alertas abiertas: 1330
atareao/reto-python  112  (critical=8 high=63 medium=41 low=0 unknown=0)
atareao/bloc  84  (critical=0 high=46 medium=29 low=9 unknown=0)
atareao/presu  79  (critical=1 high=45 medium=29 low=4 unknown=0)
atareao/tareas  59  (critical=0 high=32 medium=22 low=5 unknown=0)
atareao/asistentes  55  (critical=3 high=19 medium=30 low=3 unknown=0)
```

## Configuration

The execution config lives in `~/.config/ghwatch/config.toml` and **must keep
`600` permissions**: it holds credentials in clear text. If the file is more
permissive than `600`, `ghwatch` rejects it with a warning and does **not** use it
(falling back to the environment and defaults).

`ghwatch init` writes this template (all keys commented out) only if the file
does not already exist:

```toml
# Configuración de ghwatch.
#
# Este fichero contiene credenciales en claro: debe conservar permisos 600.
# Las variables de entorno tienen prioridad sobre estos valores.
#
# github_token = "ghp_..."
# valet_base_url = "http://localhost:3000"
# valet_ingest_token = "val_..."
# db_path = "~/.local/share/ghwatch/ghwatch.db"
# report_dir = "~/.local/share/ghwatch/reports"
# concurrency = 6
# include_forks = false
# include_archived = false
```

Keys:

| Key | Type | Default | Meaning |
|---|---|---|---|
| `github_token` | string | — | GitHub credential. Prefer `GITHUB_TOKEN`. |
| `valet_base_url` | string | `http://localhost:3000` | Valet base URL. |
| `valet_ingest_token` | string | — | Valet ingest key (`val_…`). |
| `db_path` | path | `~/.local/share/ghwatch/ghwatch.db` | SQLite database. |
| `report_dir` | path | `~/.local/share/ghwatch/reports` | Report directory. |
| `concurrency` | integer | `6` | GitHub client concurrency. |
| `include_forks` | bool | `false` | Include forks in the inventory. |
| `include_archived` | bool | `false` | Include archived repositories. |
| `github_base_url` | string | `https://api.github.com` | GitHub API base (for testing). |
| `osv_base_url` | string | `https://api.osv.dev` | OSV base (for testing). |

`valet_base_url`/`valet_ingest_token` are also accepted under the aliases
`base_url`/`ingest_token`. Unknown keys are ignored. Paths starting with `~` are
expanded against `$HOME`.

Environment variables:

| Variable | Overrides key |
|---|---|
| `GITHUB_TOKEN` | `github_token` |
| `VALET_BASE_URL` | `valet_base_url` |
| `VALET_INGEST_TOKEN` | `valet_ingest_token` |
| `GHWATCH_DB` | `db_path` |
| `GHWATCH_REPORT_DIR` | `report_dir` |
| `GHWATCH_CONCURRENCY` | `concurrency` |
| `GHWATCH_INCLUDE_FORKS` | `include_forks` |
| `GHWATCH_INCLUDE_ARCHIVED` | `include_archived` |
| `GHWATCH_GITHUB_BASE_URL` | `github_base_url` |
| `GHWATCH_OSV_BASE_URL` | `osv_base_url` |
| `RUST_LOG` | Log filter (overrides `--quiet`/`-v`). |

Booleans accept `1/true/yes/on` and `0/false/no/off`.

**Precedence:** command-line flags > environment variables > config file >
built-in defaults.

## GitHub credential

The token is resolved in this order:

1. The **`GITHUB_TOKEN`** environment variable.
2. Otherwise, the **`gh`** CLI: `gh auth token`.
3. Otherwise, an explanatory error and exit code `3`.

The required scope is the classic **`repo`**. Fine-grained tokens work too, as
long as they grant **Dependabot alerts: read** and **Secret scanning alerts:
read** for the repositories you want to watch. The token is never logged or
printed: it is redacted in `Debug`/`Display` and never travels inside an error.

## Valet notification

`ghwatch` sends a *best-effort* summary to Valet through its ingest API. It does
**not** modify Valet in any way.

**1. Create an ingest key.** The key management routes require a Valet OIDC
session cookie; the token is shown **only once**, so copy it right away:

```bash
curl -i -X POST http://localhost:3000/api/ingest-keys \
  --cookie "valet_session=<your-session-cookie>" \
  -H "Content-Type: application/json" \
  -d '{ "name": "ghwatch" }'
# 201 -> { "id": "…", "name": "ghwatch", "prefix": "val_XXXXXXXX", "token": "val_…" }
```

**2. Store it.** Put the `val_…` token in `~/.config/ghwatch/config.toml`
(`valet_ingest_token`, still `600`) or export `VALET_INGEST_TOKEN`.

**3. Send.** `scan` notifies on its own; `notify` resends the latest local report:

```bash
ghwatch scan                 # scan, write the report and notify if there is news
ghwatch scan --force         # notify even without changes
ghwatch notify               # resend the latest report, no scanning
```

By default the notification follows the **only-on-change** rule: it is sent only
when `new > 0` or `resolved > 0` (this avoids a daily "nothing happened" message).
`--force` bypasses that rule. A Valet failure is **best-effort**: it never changes
the exit code, and the report on disk is already written. The `message` is capped
at 4096 bytes (trimmed on UTF-8 boundaries, never splitting a character) and the
`metadata` at 8192 bytes, always referencing the report path on disk.

**Verify** that the notification arrived with the inbox (OIDC session required):

```bash
curl -i "http://localhost:3000/api/notifications/unread?limit=5" \
  --cookie "valet_session=<your-session-cookie>"
```

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success. Also covers "Valet is down" and "nothing new". |
| `2` | Usage error (emitted by `clap`). |
| `3` | No usable GitHub credential. |
| `4` | Fatal GitHub API failure. |
| `5` | Fatal local failure: database, report or I/O. |

## Operation

A **systemd user timer** runs a daily scan at 07:00 (`Persistent=true`, so a
missed run happens after boot). The units live in `systemd/`:

```bash
mkdir -p ~/.config/systemd/user
cp systemd/ghwatch.service systemd/ghwatch.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now ghwatch.timer

# Run even when no session is open
loginctl enable-linger "$USER"
```

If the binary is not in `~/.local/bin/ghwatch`, adjust `ExecStart` in
`ghwatch.service` (for example `%h/.cargo/bin/ghwatch scan`). Check the timer and
its logs with:

```bash
systemctl --user list-timers ghwatch.timer
journalctl --user -u ghwatch
```

The [`justfile`](justfile) wraps the common operations:

```bash
just --list        # list the recipes
just scan          # cargo run --release -- scan
just report --format markdown
just check         # fmt-check + clippy + tests
just build-release # cargo build --release --locked
just install-timer # copy the units and enable the timer
just validate      # openspec validate --specs --strict
```

The CLI recipes run `cargo run --release -- …` by default; after `just install`
you can point them at the installed binary with
`export GHWATCH_BIN=$(command -v ghwatch)`.

## Development

The quality gate is `just check` (or the equivalent directly):

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- **Tests never use the network or credentials.** GitHub, OSV and Valet are
  simulated with [`wiremock`](https://crates.io/crates/wiremock); SQLite runs
  **in memory** (`sqlite::memory:`, one connection) and migrations are embedded
  with `include_str!` and applied at runtime (no `DATABASE_URL`, no `query!`).
- **Test convention:** each file in `tests/` is wrapped in a module named after
  the file (for example `mod github { … }` in `tests/github.rs`), so
  `cargo test github` selects only that group.
- **Language:** source comments and user-facing strings are in Spanish;
  structural headings and `SHALL`/`MUST` in the spec stay in English.

The project is built with two quality gates and an audit loop:

- **SDD with OpenSpec** — no code or test is written before an approved change
  exists in `openspec/changes/`. `openspec/` is the living spec: contracts,
  *Given/When/Then* scenarios and a `tasks.md`. `PLAN.md` is the architectural
  checkpoint and `AGENTS.md` describes the current state and invariants.
- **Strict TDD** — every task is red → green → refactor, running the suite from
  the CLI at each step.
- **Independent audit** — each phase is reviewed before it is considered closed,
  and the findings are applied with tests too.
- **Gitflow with gitmoji** — `main` and `development` only receive changes
  through pull requests; day-to-day work happens on `feature/*` branches. See
  [`GIT_FLOW.md`](GIT_FLOW.md) for branches, commits and the release flow.

## Architecture

```
src/
  main.rs   # entrypoint: configures tracing (stderr) and delegates to cli::run
  cli.rs    # clap: scan | report | diff | history | repos | notify | init
  config.rs # config.toml + env + precedence and permission checks
  auth.rs   # GITHUB_TOKEN -> `gh auth token` -> typed error
  github/   # HTTP client, Link pagination, rate limit, the three alert sources
  domain/   # pure types: Alert, Severity, ScanRun, Drift, Priority
  scan.rs   # bounded-concurrency orchestration and progress
  osv/      # OSV.dev querybatch client, ecosystem mapping, merge
  store/    # sqlx migrations, idempotent upsert, lifecycle and drift
  report/   # console, Markdown and versioned JSON, plus the repository listing
  notify/   # Valet ingest client and report/notify coordination
```

`domain` is pure and testable with no network or database, and `store` depends on
`domain` (never the reverse). Reports are written **before** notifying; a Valet
failure never leaves a half-written report. The measured reconnaissance against
the author's account found **367 active repositories** with roughly **1.333 open
alerts across 90 repositories**.

## License

MIT. Author: **Lorenzo Carbonell** (`atareao`) · <https://atareao.es>.

See the full text in [`LICENSE`](LICENSE).
