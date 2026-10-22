# Product

## Register

product

## Users

Three overlapping groups, all working long sessions at a desk:

- **Oracle developers and DBAs** moving over from Oracle SQL Developer. They expect its layout, menus and shortcuts (Ctrl/Cmd+Enter, F5, F10, F11/F12) to work without relearning.
- **Multi-database developers** who switch between PostgreSQL, MySQL, SQL Server, SQLite, DuckDB and ClickHouse the way they would in DBeaver.
- **Data analysts** who mostly run SELECTs, scan result grids and export CSV/JSON.

Primary task on any screen: write and run SQL, read results, inspect an object.

## Product Purpose

A native (Cocoa on macOS, Win32 on Windows) SQL Developer: the same workflow and screen layout, without a JVM, with DBeaver-style on-demand native driver downloads. Success = a SQL Developer user feels at home within a minute and the app stays fast with large results.

## Brand Personality

Familiar, dense, calm. A working tool, not a showcase: information-dense, quiet chrome, color only where it carries meaning (run = green, errors = red, syntax highlighting).

## Anti-references

- Airy SaaS/web dashboards: big padding, cards, rounded hero panels, gradients.
- Decorative color and playful illustration.
- Anything that moves SQL Developer's controls to unfamiliar places.

## Design Principles

1. **Familiarity first**: SQL Developer's layout, labels and shortcuts win over novelty.
2. **Density with order**: show a lot, but align it; spacing is consistent, not generous.
3. **The data is the hero**: grids and editor get the space; chrome stays out of the way.
4. **Every state speaks**: running, fetched N rows, errors with line numbers, empty states that tell you what to do next.
5. **Native where it counts**: OS dialogs, menus, keyboard behavior.

## Accessibility & Inclusion

WCAG 2.2 AA: text contrast ≥ 4.5:1, every icon button has a tooltip and a menu/shortcut equivalent, full keyboard operation, no meaning carried by color alone (row edit states also marked in the row header).
