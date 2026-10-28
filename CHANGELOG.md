# Changelog

## v0.7.0 — 2026-10-28

### Features
- **Connections tree**: Apply Filter / Clear Filter on object folders (name patterns, saved per connection); package members, procedure / function arguments and table partitions as child nodes; New … templates on every folder; Edit… for PL/SQL, views and sequences; Run…, Rename…, Enable / Disable, Refresh Data, Test (database links), Compile Invalid, Gather Statistics; Quick DDL to worksheet, clipboard or file.
- Oracle folders: Directories, Public Synonyms, Scheduler Jobs (run, stop, enable, disable, run log) and Recycle Bin (flashback, purge). MySQL / SQL Server: Indexes folder.
- **Oracle users**: New User, Edit User (password, tablespaces, quota, lock / unlock, expire), Grant, Revoke, Drop User. Passwords never appear in confirmations, errors, worksheets or history.
- Connections: Rename, Duplicate, Reconnect, Disconnect All; colour per connection (e.g. red for production); startup SQL run on every new session; TNS aliases from tnsnames.ora to pick; Oracle external authentication (`auth=external`); an expired Oracle password can be changed at login; Export / Import connections file.
- **Uncommitted changes**: disconnecting, deleting or editing a connection and quitting ask to commit or roll back first.
- **Editor**: Toggle Line / Block Comment, Go to Line, F9, SQL History on Ctrl/Cmd+Up / Down, code templates (abbreviation + Tab), Popup Describe (Shift+F4), Format to Clipboard, Extended Paste, incremental find, Print, Close All, Rename Tab, next / previous tab keys.
- **SQL History window** with connection, type and time, filter, Append / Replace / Clear.
- **Script runner**: SET FEEDBACK / TIMING / ECHO / HEADING / NULL / AUTOCOMMIT, WHENEVER SQLERROR EXIT, EXIT / QUIT; double-click an error in Script Output to jump to its line.
- **Result grid**: Filter Column by distinct values; Export… dialog (CSV / delimited, Excel, JSON, INSERT, HTML, XML; header, delimiter, selected rows, file or clipboard); Copy Selected Column Header(s); Pin a result in its own tab; Show SQL; Run ▸ Cancel stops a running export.
- **Tools**: Reports (version, storage, invalid objects, tables without a primary key, unindexed foreign keys, users, locks, waits, …), Data Compare, Database Export, Database Copy, ER Diagram (SVG), Search Source Code, Explain Plan (DBMS_XPLAN).
- Create / Edit Table: primary key Enabled flag (Oracle).
- Preferences: Display NULL as, tnsnames directory, open a worksheet on connect, SQL History limit, line numbers, NLS timestamp format, DDL storage clauses, save bind values, reopen worksheets at start, code templates.
- Worksheets left open at quit reopen at the next start; bind values are remembered; View ▸ Log, Recent Objects; File ▸ Recent Files.

### Bug fixes
- Oracle: Drop on a database link failed (the link name was schema-qualified).
- Quick DDL of Oracle packages, procedures and functions gave bare source text; it is now runnable CREATE OR REPLACE … / DDL.
- Renaming a connection left its Data Pump file records under the old name.

## v0.6.0 — 2026-10-28

### Features
- **SQL editor**: incremental syntax highlighting; aliases highlighted, Ctrl/Cmd+Click on an alias selects the table it stands for.
- Holding Ctrl (Cmd on macOS) shows object names as clickable links; opening an object (Ctrl+Click, Find DB Object) also selects it in the Connections tree.
- **INSERT column tip**: in a VALUES tuple or the column list, a tip shows the table's columns with the one being typed highlighted; column autocomplete in the INSERT column list.
- Format SQL on Ctrl/Cmd+Shift+F (Ctrl/Cmd+F7 still works).
- SQL highlighting in every SQL text view: Create / Edit Table DDL, Data Pump previews, Script Output, DBMS Output, Messages - Log and every confirmation showing the exact statement.
- Result grid: Edit Value pencil, Alt+Enter, double-click opens long values.
- Engine icons in the connection dialog's Database Type list, the drivers list and connection lists.
- Splash screen at startup; a little database in the status bar hops while the database works.
- Faster result grids: fetch all, paste, filters and sorting; new rows in the data editor are committed 500 at a time in one INSERT.

### Bug fixes
- Fixed a crash on quit (macOS) when an editor popup was open.
- Fixed a crash in the editor when editing large scripts with non-ASCII text.
- SQLite: expression and CTE columns with numbers now sort numerically.
- SQLite: Commit no longer fails with "database is locked" after browsing metadata; PostgreSQL: metadata queries no longer hold a transaction open.

## v0.5.0 — 2026-10-27

### Features
- **Data editor like SQL Developer**: Insert Row after the cursor row (Ctrl+I / Cmd+I), Duplicate Row, Delete Selected Row(s) (Ctrl+D / Cmd+D), Set to NULL, Revert Row, Paste of copied cells (tab-separated, from Excel too), Single Record View, Sort… on several columns (server side, Nulls First / Last). Commit (F11), Rollback (F12) and Refresh (Ctrl+R / Cmd+R) work in a table's Data tab.
- **Messages - Log** under the Data tab: every statement posted, errors as "Row n: …", commit and rollback.
- Preference **Post edits on row change**: a row goes to the database when the cursor leaves it; Commit then only commits.
- **Editable query results**: a worksheet query of one table (no joins, grouping or subqueries) can be edited in the grid and posted with Commit.
- View Value: date picker for DATE / TIMESTAMP cells.
- **Create / Edit Table** redone after SQL Developer: Columns with Column Properties (BYTE / CHAR units, Default on NULL, virtual columns, identity columns with start, increment, min, max, cache, cycle, order), Primary Key with column order, Unique Constraints, Foreign Keys (referenced table list, column pickers, On Delete), Check Constraints, Indexes with an expression picker, Storage Options (Oracle), table comment and temporary tables. Existing constraints can be renamed and enabled / disabled.
- **Table submenus** in the Connections tree: Table ▸ (rename, copy with or without data, drop with cascade / purge, truncate, lock, comment, parallel, logging, count rows), Column ▸ (add, rename, drop, comment), Constraint ▸ (add primary key / foreign key / unique / check, enable / disable single, all or related foreign keys, rename, drop), Index ▸ (create, drop, rebuild online), Trigger ▸ (template, PK from sequence, enable / disable, drop). Each shows the exact SQL before it runs.

### Bug fixes
- Uncommitted grid edits are no longer lost silently: running another statement, refreshing / filtering / sorting the Data tab or closing the worksheet asks Commit, Discard or Cancel.
- MySQL / SQL Server: closing or refreshing a large, partly fetched result no longer downloads every remaining row before the next statement runs.
- MySQL: a result grid left idle for more than a minute no longer loses the connection.
- PostgreSQL (autocommit): an idle result grid no longer keeps a transaction open with its locks; fetching continues when the grid is scrolled again.
- The data editor shortcuts shown in tooltips (Ctrl+I, Ctrl+D, F11, F12) now work in the Data tab.

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
