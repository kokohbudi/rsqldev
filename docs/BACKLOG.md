# Backlog

Ordered by value. Move an item into a spec (`docs/superpowers/specs/`) when work starts.

## Features
1. **Space Manager & Optimize** (Toad-style, Oracle first) — tablespaces / datafiles (size, used, free, autoextend), largest segments per tablespace; table size incl. indexes / LOBs / partitions, rows, stats age, wasted space, chained rows, reorg score; actions shrink space, move / rebuild (+ rebuild UNUSABLE indexes), rebuild index online, Segment Advisor — each with Execute or Generate Script. (Read-only parts exist as Tools ▸ Reports; Gather Statistics is on the Table menu.)
2. **Roles** — list roles with their grantees; create / drop role. (Users, grants and revokes are on the tree's Other Users nodes.)
3. **User-Defined Reports** — saved parameterised queries with simple charts, shareable as files. (Shipped reports: Tools ▸ Reports.)
4. **Editor folding** — needs SetMarginMask / SetMarginSensitive, which wxDragon 0.9.23 does not expose.
5. **Explain Plan as a tree** (expand / collapse) instead of the indented grid; Autotrace V$MYSTAT statistics.
6. **Script runner VARIABLE / PRINT** (real OUT binds) and CONNECT.
7. **AWR mode for Top SQL** — `DBA_HIST_SQLSTAT` per snapshot range, only where the Diagnostics Pack is licensed.
8. **"Ready for 1.0" checklist** — config formats stable with migrations, no known crash / data-loss bugs, core features tested on all three OSes against real databases.

## Data Pump follow-ups (deferred during review)
- Records greyed when a connection is deleted; Download by Name also offers Delete (spec gaps).
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
