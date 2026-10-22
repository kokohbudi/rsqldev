# rsqldev

Native SQL Developer clone in Rust. wxWidgets (via wxDragon) renders real Cocoa widgets on
macOS and Win32 widgets on Windows. Multi-database like DBeaver, with on-demand native driver
download (Oracle Instant Client) instead of JDBC.

## Run

    cargo run --release -p rsqldev

## Layout

- `core/` – backends (Oracle, PostgreSQL, MySQL, SQL Server, SQLite, DuckDB, ClickHouse),
  driver registry (`core/drivers.toml`, 40 drivers), downloader, dialect metadata, script runner, DML.
- `app/` – UI: connections navigator, SQL worksheet, object viewer, data editor, driver manager.
- `docs/feature-catalog.md` – SQL Developer 26.2 feature spec (MVP / P2 / P3).

## Tests

    cargo test -p rsqldev-core                         # unit + SQLite/DuckDB
    docker compose -f docker-compose.test.yml up -d
    cargo test -p rsqldev-core --test backends -- --ignored --test-threads 1

UI smoke test against a saved connection:

    RSQLDEV_CONFIG_DIR=/tmp/cfg RSQLDEV_SELFTEST="<connection name>" RSQLDEV_SELFTEST_PW=<pw> cargo run -p rsqldev
