# Backlog

Ordered by value. Move an item into a spec (`docs/superpowers/specs/`) when work starts.

## Features
1. **Space Manager & Optimize** (Toad-style, Oracle first) — tablespaces / datafiles (size, used, free, autoextend), largest segments per tablespace; table size incl. indexes / LOBs / partitions, rows, stats age, wasted space, chained rows, reorg score; actions shrink space, move / rebuild (+ rebuild UNUSABLE indexes), rebuild index online, gather stats, Segment Advisor — each with Execute or Generate Script.
2. **Users, Roles & Privileges** — list users / roles / grants; create / alter user (password, default tablespace, quotas, lock / unlock, expire); grant / revoke with DDL preview.
3. **Scheduler Jobs** (`DBMS_SCHEDULER`) — list, run / stop, enable / disable, run history with errors, create job dialog.
4. **Data Compare** — compare table rows between schemas / connections by primary key; script INSERT / UPDATE / DELETE to sync.
5. **User-Defined Reports** — saved parameterised queries with simple charts, shareable as files (SQL Developer Reports).
6. **ER Diagram** — tables and FK relations of a schema, export PNG / SVG.
7. **AWR mode for Top SQL** — `DBA_HIST_SQLSTAT` per snapshot range, only where the Diagnostics Pack is licensed.
8. **"Ready for 1.0" checklist** — config formats stable with migrations, no known crash / data-loss bugs, core features tested on all three OSes against real databases.

## Data Pump follow-ups (deferred during review)
- Connection rename → `datapump::rename_conn`; records greyed when a connection is deleted; Download by Name also offers Delete (spec gaps).
- Reconnect retry: status note that the earlier uncommitted transaction was lost.
- Modal log viewer pauses all tickers while open → modeless viewer.
- Files tab runs one query per record (+ per `%U` piece) every 5 s → refresh less often or on demand.
- Refresh right after Delete can be skipped while a probe is in flight.
- Missing `%U` piece 01 shows `X_01.dmp` instead of `X_%U.dmp`; hint with an empty directory has a double space.
- Upload: 4 MiB local read on the UI thread per chunk; gauge shows 100 % before the last chunk lands; Cancel during the last chunk; X on the progress dialog hides it.
- Import watch: wording when the lookup fails, silent 24 h give-up, temp SQL file removal.
- Export: `file_size` pattern (`^\d+[KMGT]?$`), `where_clause` word boundary, connect timeout on the short-lived session.
- `FileKind::Other` saves back as "other" (older builds lose newer kinds).

## Performance / build ideas (measure first)
- `mimalloc` global allocator (mostly Windows, large result sets).
- PGO only after a headless core benchmark exists (grid formatting, export, DDL/DML generation) and shows a gain.
- DuckDB as an optional Cargo feature for lighter local analysis (on in release builds).
