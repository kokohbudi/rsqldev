# Changelog

## v0.4.0 — 2026-10-23

### Features
- **Oracle Data Pump without SSH**: Tools ▸ Data Pump Export / Import dialogs (full, schema, table, tablespace; content, compression, parallel, encryption, remap). Jobs keep running on the server when the app is closed; the Data Pump tab lists jobs (stop / resume / log) and the dump files in the server directory. Dump files download to this computer and upload to the server over the database connection, with progress and resume.
- **Create / Edit Table** and **New Index** dialogs (SQL Developer style), with the exact DDL shown before it runs so it can also be copied and run by hand.
- **Tools ▸ Top SQL**: heaviest statements on the server (elapsed, CPU, I/O, executions).
- **Tools ▸ Schema Compare**: differences between two schemas with a sync script.
- **Run PL/SQL** (Ctrl+F10): prompts for procedure / function arguments.
- **Autotrace** (F6).
- Bind variables (`:name`, `:1`) are prompted for when a statement runs.
- Snippets: Edit ▸ Insert Snippet (Ctrl+Shift+J), Save Selection as Snippet.
- Find DB Object; File ▸ New SQL Worksheet.
- Editor: Ctrl+Click (Cmd+Click on macOS) opens the object under the caret; duplicate / move lines, matching bracket, bookmarks.
- Ctrl+C / Cmd+C copies from result grids, the connections tree and text panes on every platform.
- Result grid: the selected cell is highlighted; Delete Row removes every selected row.
- Connections tree: an icon per database engine.
- Smaller, faster release builds (link-time optimisation; macOS download 51 → 42 MB).

### Bug fixes
- DELETE / UPDATE without WHERE asks before it runs.
- Data editor: UPDATE / DELETE locate the row by ROWID instead of the visible key columns.
- Lost server sessions (DPI-1010, ORA-03113 …) reconnect and retry once, only when the statement cannot have run.
- Windows: grid row height follows the font, so rows are no longer clipped.
- Start Page links no longer open the web browser.
- Schema Compare keeps object names that appear inside comments.
