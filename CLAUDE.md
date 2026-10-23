# rsqldev — agent context

## Modes (always on)
- **Caveman mode: full.** Every reply terse. No articles/filler/pleasantries/hedging. Fragments ok. Code, commits, PRs, security warnings, destructive-op confirmations: normal prose.
- **Ponytail mode: full.** Laziest solution that works: YAGNI → stdlib → native widget/DB feature → existing dep → one line → minimal code. No speculative abstractions, no new deps for few-line jobs. Mark deliberate shortcuts `// ponytail: <ceiling + upgrade path>`. Non-trivial logic leaves one runnable check (unit test).
- Talk to user in **Bahasa Indonesia**. Code, comments, commits, docs in English.

## What
Native SQL Developer clone. Rust + wxWidgets via wxDragon 0.9.23 (real Cocoa / Win32 / GTK widgets). Multi-DB: Oracle (ODPI-C, Instant Client downloaded at runtime), PostgreSQL, MySQL, SQL Server, SQLite, DuckDB, ClickHouse. 40 drivers in `core/drivers.toml`.

## Layout
- `core/` — no UI. `backend/` (one file per engine, trait `Backend` in `lib.rs`), `worker.rs` (one thread per session, `Cmd` / `Reply`, reconnect retry), `dialect.rs` (per-engine metadata SQL, `fill()` templates `{o} {n} {q} {lq}`), `ddl.rs` (create/alter table, index), `dml.rs` (grid edits → SQL, ROWID-first keys), `datapump.rs` (Oracle Data Pump PL/SQL, server files), `compare.rs`, `script.rs` (splitter, `&` substitution, `:bind` literals), `lob.rs` (spill files, `private_dir`), `config.rs`.
- `app/` — wxDragon UI. `main.rs` (App, menus, `Page` tabs), `conn.rs` (`Conn`: work lane + metadata lane), `worksheet.rs`, `grid.rs` (`ResultGrid`), `connections.rs` (tree), `object_view.rs`, `table_editor.rs`, `datapump_*.rs`, `transfer.rs`, `sessions.rs`, `topsql.rs`, `copy.rs`, `hub.rs`.
- `docs/feature-catalog.md` — SQL Developer feature spec. `docs/superpowers/` — specs + plans. `site/index.html` — download page (GitHub Pages, English).

## Commands
- Check before every commit: `cargo check --workspace --message-format short` (<1 s warm; `target/` ≈10 GB on purpose, keep it).
- Core tests: `cargo test -p rsqldev-core` (fast: unit + SQLite/DuckDB). CI runs it on Linux.
- NEVER locally: release builds, `scripts/bundle-macos.sh` (universal), Docker / `--ignored` backend suite — slow, CI does them.
- UI self-test: `RSQLDEV_CONFIG_DIR=/tmp/cfg RSQLDEV_SELFTEST="<conn>" RSQLDEV_SELFTEST_PW=<pw> cargo run -p rsqldev`.

## Workflow
- Work on `develop`. Commit per change, push a few commits at once (each push = 4 CI jobs downloading ~0.8 GB caches; too many → cache egress limit → full rebuilds). Newer push cancels older run.
- CI: `.github/workflows/build.yml` (macOS universal, Windows x64, Linux x86_64/aarch64). Release profile = fat LTO, 1 codegen unit, stripped.
- Release only when user asks: write the `CHANGELOG.md` entry first (version + date, **Features** and **Bug fixes** since the last tag, from `git log <last-tag>..develop`, user-facing wording), merge `develop` → `main`, bump `version` in root `Cargo.toml` (`[workspace.package]`), push tag `v*` (`release.yml` publishes zips).
- Stage files by path, never `git add -A` (`.idea/` etc.).

## Patterns
- UI never blocks: workers send on channels, UI polls with `hub::every_tick(|| -> bool)` + `try_recv`. Return false to stop. A modal opened inside a tick pauses all tickers (no panic).
- Metadata / admin queries → `Conn::meta_query` (separate session). Worksheet / DDL → `Conn::work_query`. Extra long jobs (transfers) → own `worker::Session`.
- Callbacks hold `Weak` refs; tabs die → tickers stop.
- Names into SQL only via `dialect::lit` / `quote` / `fill` or validated (`datapump::valid_name`). Never raw.
- Destructive actions: confirm and show the exact statement first. DELETE/UPDATE without WHERE asks.
- Oracle PL/SQL positional binds are numbered by **first appearance** in the text — keep placeholders in order, test it.
- `Bind::Text` binds as CLOB: assign to typed locals (`x VARCHAR2(n) := :1`) before overloaded calls.
- Secrets (passwords) never in preview text, worksheet, history, status bar, errors.

## wxDragon gotchas (past CI failures)
- `SpinCtrl::value()` not `get_value()`. `SizerFlag::empty()` not `0`. Check APIs in `~/.cargo/registry/src/*/wxdragon-0.9.23/src`.
- No `FindFocus`; grid focus is on an inner window → copy target via `copy::set`, programmatic moves inside `copy::quietly`.
- HyperlinkCtrl opens its label as URL (no skip) → use StaticText.
- `oracle` crate 0.6.3: `Bfile` is private → read server files via PL/SQL `LOADBLOBFROMFILE` chunks.
- Don't move a value into a callback while borrowing it as an argument (E0505) — clone first.
