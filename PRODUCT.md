# Product

<!-- impeccable:product-schema 1 -->

## Platform

adaptive

Desktop, not mobile: one native app for macOS (universal, 11+), Windows 10/11 x64 and Linux (GTK 3, x86_64 and ARM64), rendered with each OS's own widgets (Cocoa, Win32, GTK) through wxWidgets / wxDragon. "Adaptive" here means it follows each desktop OS's conventions (menus, dialogs, keyboard, Keychain / Credential Manager / Secret Service); iOS and Android guidance does not apply.

## Users

Public, open-source users who download builds from the GitHub Pages site and GitHub Releases. Three overlapping groups, all working long sessions at a desk:

- **Oracle developers and DBAs** moving over from Oracle SQL Developer. They expect its layout, menus and shortcuts (Ctrl/Cmd+Enter, F5, F10, F11/F12) to work without relearning.
- **Multi-database developers** who switch between PostgreSQL, MySQL, SQL Server, SQLite, DuckDB and ClickHouse the way they would in DBeaver.
- **Data analysts** who mostly run SELECTs, scan result grids and export CSV/JSON.

Primary task on any screen: write and run SQL, read results, inspect an object.

## Product Purpose

A native SQL Developer: the same workflow and screen layout, without a JVM, with DBeaver-style on-demand native driver downloads. Success = a SQL Developer user feels at home within a minute and the app stays fast with large results.

## Positioning

SQL Developer's workflow, menus and shortcuts, as a native app with no JVM (light, fast with large results), and multi-database with drivers downloaded when first needed. The combination is the point: SQL Developer is Java and Oracle-first; DBeaver and DataGrip are multi-database but neither native nor SQL Developer-shaped.

## Operating Context

- Long desk sessions against real, often production, databases: destructive statements are confirmed and shown first; passwords stay in the OS secret store and never appear in text the app shows or saves.
- Corporate networks: SSH tunnels, TNS names, Oracle Instant Client downloaded from oracle.com on first Oracle connect (or an existing install pointed at).
- Distribution: zips / tarball from GitHub Releases. The macOS app is not notarized and Windows builds are unsigned, so first launch goes through Gatekeeper "Open" / SmartScreen "Run anyway" (documented on the download page).
- Several worksheets and long jobs (exports, Data Pump, ER diagrams, transfers) run side by side; long jobs use connections of their own so browsing and worksheets stay responsive.

## Capabilities and Constraints

- Engines: Oracle (ODPI-C), PostgreSQL, MySQL, SQL Server, SQLite, DuckDB, ClickHouse, plus compatibles; 40 drivers in `core/drivers.toml`.
- Feature scope tracks Oracle SQL Developer 26.2, specified in `docs/feature-catalog.md` (MVP / P2 / P3).
- Clean-room: SQL Developer's behavior is studied (a decompiled reference lives outside this repo, `~/myProjects/sqldeveloper-src`), never its code, icons or text. Original artwork only.
- Performance first: features must cost about nothing when idle; no fake delays, no idle work, work happens off the UI thread.
- License: GNU GPL v3 only (`LICENSE`): open source; anyone may use, change, share and even sell it, but whoever distributes it must pass on the source under the GPL, so closed forks are not possible.

## Brand Commitments

- Name: **rsqldev** (lower case). App icon: `app/assets/icon.svg`, `rsqldev.icns`, `rsqldev.ico`.
- Personality: familiar, dense, calm. A working tool, not a showcase: information-dense, quiet chrome, color only where it carries meaning (run = green, errors = red, syntax highlighting). One small playful touch exists and stays small: the activity "buddy" (a hopping database) in the status bar and the running worksheet's toolbar.
- Anti-references: airy SaaS/web dashboards (big padding, cards, rounded hero panels, gradients); decorative color and playful illustration beyond the buddy; anything that moves SQL Developer's controls to unfamiliar places.
- User-facing copy and docs in English.

## Evidence on Hand

- Download and getting-started page: `site/index.html` (GitHub Pages).
- Release history: `CHANGELOG.md` and tags (v0.5.0 to v0.7.0); builds on GitHub Releases.
- Feature spec: `docs/feature-catalog.md`; backlog: `docs/BACKLOG.md`.
- Absent, never to be invented: testimonials, user counts, company logos, reviews. The site's "roughly 4–8× less memory" has no measurement in the repo; measure before repeating or extending it.

## Product Principles

1. **Familiarity first**: SQL Developer's layout, labels, behavior and shortcuts win over novelty.
2. **The data is the hero**: grids and editor get the space; chrome stays out of the way; show a lot, but aligned.
3. **Every state speaks**: running (and for how long), fetched N rows, errors with line numbers, empty states that say what to do next.
4. **Native where it counts**: OS dialogs, menus, keyboard behavior, secret stores.
5. **Costs nothing when idle**: no background polling or animation without work in flight; long jobs never block the UI or the shared sessions.

## Accessibility & Inclusion

WCAG 2.2 AA: text contrast ≥ 4.5:1, every icon button has a tooltip and a menu/shortcut equivalent, full keyboard operation, no meaning carried by color alone (row edit states also marked in the row header).
