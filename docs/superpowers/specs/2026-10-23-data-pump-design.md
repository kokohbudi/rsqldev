# Data Pump (export / import) without SSH — design

Date: 2026-10-23 · Status: approved by the user ("lanjut") with the defaults below

**User's answers:** targets are on-premise Oracle 12.2+, single instance or RAC, any edition;
Autonomous Database and AWS RDS are out of scope for now (their file transfer differs).
Testing: manual, on the user's own Oracle instance, from a checklist the builder writes; no
Oracle job is added to CI.

## Intent

The user administers Oracle databases with rsqldev and wants Toad-style admin features. They
need Oracle Data Pump exports and imports (what `expdp` / `impdp` do) **from the desktop app,
with the dump files moved between the server and their laptop over the database connection
itself, never over SSH** (they often have no OS access to the database host).

Long exports must keep running on the server when the app is closed or the connection drops;
the dump is downloaded later.

## Goals

1. **Export**, every Data Pump mode: full database, one or more schemas, chosen tables,
   tablespaces. Options: include / exclude object types, a WHERE filter per table, metadata only
   or data only, a consistent point in time (`FLASHBACK_TIME = SYSTIMESTAMP`), parallel degree
   with file size per dump file (`%U` names), an estimate-only run. Dump and log files are
   written to a server DIRECTORY object (default `DATA_PUMP_DIR`, pickable from
   `ALL_DIRECTORIES`).
2. **Import**: from a dump uploaded from the laptop, or from a file already in a server directory.
   Modes full / schemas / tables; remap schema and tablespace; table exists action (skip, append,
   truncate, replace); SQLFILE mode (write the DDL to a .sql file instead of importing).
3. **Jobs monitor**: Data Pump jobs on the server from `DBA_DATAPUMP_JOBS` (fallback
   `USER_DATAPUMP_JOBS` without DBA privileges), including jobs started outside the app by
   `expdp` / `impdp`. Attach to a job for progress (percent done, current object, errors);
   Stop (resumable) and Kill. This works after the app was closed or the connection dropped.
4. **Exports list** (local): every export started from the app is recorded in the rsqldev config
   (connection, job name, directory, dump file names, log file name, started at). The list shows
   the job state and whether each file still exists on the server (`DBMS_LOB.FILEEXISTS`), with
   **Download**, **View Log** and **Delete from server**.
5. **Download a file by name**: for dumps made outside the app, the user types directory + file
   name(s) and downloads them the same way.
6. **Transfer without SSH**:
   - Download: read the server file through `BFILENAME(dir, file)` in chunks
     (`DBMS_LOB.SUBSTR` on the BFILE, about 1 MB per round trip) and append to a local file,
     streaming (never the whole file in RAM), with progress and cancel. A failed download can be
     resumed from the local file's size.
   - Upload: write the local file into the server directory with `UTL_FILE.FOPEN(..., 'wb')` +
     `PUT_RAW` in chunks (32 KB per call, several per round trip), with progress and cancel.

## Non-goals

- Engines other than Oracle (PostgreSQL pg_dump, MySQL mysqldump, ...): not now.
- Listing every file in a server directory: Oracle has no supported SQL for it; the local exports
  list plus "download by name" cover the need (user's choice).
- RMAN backups, transportable tablespaces, network-link (`NETWORK_LINK`) imports.
- Scheduling exports.

## Anti-goals

- Anything that needs SSH, an OS account on the server, or the `expdp` binary on the laptop.
- Reading a whole dump into memory (dumps reach many GB).
- Silently using licensed options: compression (`COMPRESSION=ALL`/`DATA_ONLY`, Advanced
  Compression Option) and encryption (Advanced Security Option) must be opt-in and labelled as
  licensed; metadata-only compression is the default and needs no license.
- An import that overwrites data without saying so: REPLACE / TRUNCATE are shown in the
  confirmation along with the PL/SQL that will run.

## Constraints and requirements on the database side

- Privileges, reported in the UI when missing (the way the Sessions monitor already does):
  `READ, WRITE ON DIRECTORY <dir>`; EXECUTE on `DBMS_DATAPUMP` (public by default) and
  `UTL_FILE` (often revoked from PUBLIC on hardened databases, needed for upload and delete);
  `DATAPUMP_EXP_FULL_DATABASE` / `DATAPUMP_IMP_FULL_DATABASE` for other schemas or full mode.
- Server disk space for the dump.
- Transfer speed is bounded by the SQL connection; fine for hundreds of MB to a few GB.

## How (decided)

- **PL/SQL API, not the binaries**: `DBMS_DATAPUMP.OPEN / ADD_FILE / METADATA_FILTER /
  DATA_FILTER / METADATA_REMAP / SET_PARAMETER / SET_PARALLEL / START_JOB`, then `DETACH`, so
  the job lives on in the server. The generated PL/SQL block is shown before it runs, with Run
  and Copy to Worksheet, like the table editor.
- **Job names** are unique and recognisable: `RSQ_EXP_YYYYMMDD_HH24MISS` /
  `RSQ_IMP_...`; file names derive from the job name unless the user sets them.
- **Progress**: `DBMS_DATAPUMP.ATTACH` + `GET_STATUS` (percent done, work units, errors) on the
  metadata session, polled every few seconds while the monitor tab is open, then `DETACH`.
  Stop = `STOP_JOB(immediate => 0, keep_master => 1)`; Kill = `STOP_JOB(immediate => 1,
  keep_master => 0)`, after confirming.
- **Transfers run on their own connection thread** (an extra session like Unshared Worksheet),
  so a long download blocks neither the worksheets nor the metadata queries. The local file is
  written as `<name>.part` and renamed when complete; resume continues from its size.
- **UI**: Tools ▸ Data Pump ▸ Export…, Import…, Jobs & Files. Export and Import are dialogs
  (tabs: what / options / files / PL/SQL), Jobs & Files is a document tab with two grids (server
  jobs, local exports) like the Sessions monitor.
- Code follows the existing layout: SQL and PL/SQL generation in `core` (unit-tested, like
  `ddl.rs` / `compare.rs`), dialogs and tabs in `app`.

## Decided after the builder check (designer's calls, not the user's)

- **Server files the app creates are all tracked**, not only exports: uploaded dumps, import
  logs and SQLFILE output go into the same local list, each with Download / View / Delete from
  server. "Download by name" also offers Delete. After a successful import the app offers to
  delete the uploaded dump. A SQLFILE result is downloaded and opened in a worksheet.
- **Compression and encryption are in scope** as opt-in options labelled with their license
  (Advanced Compression / Advanced Security). The encryption password is never written into
  the PL/SQL preview, worksheet or autosave: the preview shows `:password`; Run binds the real
  value; Copy to Worksheet leaves `:password` so the worksheet's bind prompt asks for it. Import
  asks for the password when the dump is encrypted.
- **Jobs monitor actions**: Stop (keep master), Resume (ATTACH + START_JOB, optional new parallel
  degree), Kill, and Clean Up for NOT RUNNING jobs whose master table is left behind (drop it,
  after confirming).
- **RAC**: jobs are started with `CLUSTER = 0` so every dump file is written by the instance the
  transfer session can read. **Standard Edition**: the parallel option is disabled (detected from
  `V$VERSION`). Version floor 12.2 (the app already relies on 12.2 views); `%L` file names only
  on 21c+, `%U` otherwise.
- **Exports list records** are keyed by connection name plus the database identity
  (`DB_UNIQUE_NAME` and DBID), follow a connection rename, are kept (greyed) when the connection is
  deleted, and show for any connection to the same database. A job started from Copy to Worksheet
  is not recorded: it shows in the Jobs monitor while it runs, and its files can be fetched by name.

## Left to the builder

- Exact dialog layout and wording.
- Chunk sizes and the polling interval, within the ranges above.
- Where the exports list is stored in the config folder, and its format.
- How the LOB / RAW chunk is fetched through the existing backend (a new worker command for
  streaming a BFILE to a local file is expected; the current result-grid path keeps large values
  out of RAM but is not meant for multi-GB files).
