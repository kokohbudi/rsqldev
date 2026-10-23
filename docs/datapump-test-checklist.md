# Data Pump manual test checklist

Run against a real Oracle 12.2+ (on-premise, single instance or RAC). Nothing here runs in CI.
Each line is an action followed by the expected result. Menu: **Tools > Data Pump Export... / Data Pump Import... / Data Pump Jobs & Files...** (Oracle connections only).

## Setup

Run as a DBA (`SYS`/`SYSTEM`):

```sql
-- scratch schema to import into
CREATE USER dp_scratch IDENTIFIED BY "Scratch#123" QUOTA UNLIMITED ON users;
GRANT CREATE SESSION, CREATE TABLE TO dp_scratch;

-- directory object (the OS path must exist and be writable by the oracle OS user)
CREATE OR REPLACE DIRECTORY dp_test_dir AS '/u01/app/oracle/dp_test';

-- non-DBA test user with a few tables to export
CREATE USER dp_user IDENTIFIED BY "User#123" QUOTA UNLIMITED ON users;
GRANT CREATE SESSION, CREATE TABLE, CREATE VIEW, CREATE PROCEDURE TO dp_user;
GRANT READ, WRITE ON DIRECTORY dp_test_dir TO dp_user;
GRANT READ, WRITE ON DIRECTORY data_pump_dir TO dp_user;
GRANT EXECUTE ON sys.utl_file TO dp_user;      -- upload and delete
-- DBMS_DATAPUMP is EXECUTE to PUBLIC by default

-- DBA-level test user (other schemas, full mode, DBA_DATAPUMP_JOBS)
CREATE USER dp_dba IDENTIFIED BY "Dba#123";
GRANT DBA, DATAPUMP_EXP_FULL_DATABASE, DATAPUMP_IMP_FULL_DATABASE TO dp_dba;
GRANT READ, WRITE ON DIRECTORY dp_test_dir TO dp_dba;
```

As `dp_user` create sample data: `CREATE TABLE t_small AS SELECT level id, 'row '||level txt FROM dual CONNECT BY level <= 1000;` and a bigger `t_big` (a few hundred MB) so a job runs long enough to observe and cancel.

- [ ] Create two saved connections, `dp_user` and `dp_dba`. Both connect.
- [ ] Open the Tools menu on a non-Oracle connection. The Data Pump items are not offered (or say Oracle only).
- [ ] Note the server file size for each test with `SELECT DBMS_LOB.GETLENGTH(BFILENAME('DP_TEST_DIR','x.dmp')) FROM dual;` (or the Size column in Jobs & Files) for comparisons below.

## Export

- [ ] As `dp_user`, open Data Pump Export. Dialog has tabs What / Options / Files / PL/SQL. Mode Schemas lists the own schema.
- [ ] Pick own schema, directory `DP_TEST_DIR`, open the PL/SQL tab. Block shows `DBMS_DATAPUMP.OPEN`, `ADD_FILE`, `START_JOB(... cluster_ok => 0)` and `DETACH`. Job name looks like `RSQ_EXP_YYYYMMDD_HH24MISS`; files `<JOB>.dmp` and `<JOB>.log`.
- [ ] Press OK. Job starts, Jobs & Files tab opens or can be opened; the job appears in the server jobs grid as EXECUTING, then percent moves, then it leaves EXECUTING.
- [ ] After completion the local exports list shows the dump and log files with "exists on server" and the job state.
- [ ] Select the job, View Log. Log opens in a viewer and ends with "successfully completed".
- [ ] Select the dump file, Download... and pick a folder. Progress dialog moves, ends OK. The local file size equals the server size exactly.
- [ ] Tick "Consistent (FLASHBACK_TIME = SYSTIMESTAMP)" and run. The PL/SQL tab shows `FLASHBACK_TIME` with `SYSTIMESTAMP`; Oracle accepts `SET_PARAMETER` (no ORA-39001/ORA-39071), the job completes. (Only a live Oracle can confirm this.)
- [ ] Tick "Estimate only (no dump written)". Job completes, log shows the estimate, no dump file is listed or created.
- [ ] Export with a table WHERE filter (Tables mode, Add Filter, e.g. `t_small` WHERE `id <= 10`). Import it later (see Import): only 10 rows.
- [ ] Export with parallel 2 and a file size (e.g. 50 MB). The PL/SQL tab uses `<JOB>_%U.dmp`. After the job: `_01` and `_02` pieces are listed (and more if size forces it); each Downloads with its exact server size. The list stops at the first missing number.
- [ ] Metadata only, then data only. Log and dump sizes differ accordingly (metadata-only is tiny).
- [ ] Compression and Encryption controls are labelled "needs Advanced Compression" / "Needs Advanced Security". Default compression is metadata-only.
- [ ] Encrypt the dump with password `Secret#1`. The PL/SQL tab shows `:password`, never the real value. OK runs and completes. Copy to Worksheet leaves `:password` and the worksheet asks for the bind on run. The password is not in the worksheet text, autosave or config.
- [ ] As `dp_dba`, export Full and another user's schema. Both start. As `dp_user`, choosing another schema or Full gives a clear privilege hint (needs `DATAPUMP_EXP_FULL_DATABASE`), not a raw stack.
- [ ] Copy to Worksheet, run the block there by hand. Job runs and shows in the Jobs monitor while it runs. It is NOT recorded in the local exports list; its files can be fetched with Download by Name.
- [ ] Standard Edition (if available): the parallel control is disabled. On Enterprise it is enabled.

## Import

- [ ] Open Data Pump Import (tabs Source / What / Options / PL/SQL). Source: "Upload a local .dmp file first" with Browse...; or "Dump file(s) already in the directory".
- [ ] Source = file already on the server, mode Schemas, remap `DP_USER` to `DP_SCRATCH`. PL/SQL tab shows `METADATA_REMAP`. Run. Job completes; in `dp_scratch`, `SELECT COUNT(*)` on each table equals the source table.
- [ ] Source = upload: Browse... to a local .dmp, run. Upload progress dialog moves, then the import job starts and completes; row counts match. The uploaded dump is tracked in the local list; after success the app offers to delete the uploaded dump from the server. Accept: the file is gone from the server.
- [ ] Table exists action: run the same import again with Skip (nothing changes), Append (rows double), Truncate (back to original), Replace (table recreated). The confirmation names Truncate and Replace and shows the PL/SQL before anything runs.
- [ ] Remap tablespace (e.g. USERS to another existing tablespace). Objects land in the new tablespace.
- [ ] Tables mode: import only one table from a schema dump. Only that table appears.
- [ ] "Only write the DDL to a SQL file (nothing is imported)": job completes, nothing is created, the SQL file is downloaded and opens in a worksheet with the DDL. The temp folder for it is cleaned up afterwards (see Failure cases).
- [ ] Import the encrypted dump with the right password. Works. With a wrong password: a clear one-line error (ORA-39xxx about the password/decryption), no password echoed in the message. With no password while the dump is encrypted: clear error too.
- [ ] Import with a file name that does not exist in the directory. One-line hint (ORA-39002/ORA-31640/ORA-27037), not a raw stack.
- [ ] Names with spaces, quotes or `..` typed in directory/file fields are rejected by validation before anything runs.

## Jobs and files

- [ ] Open Jobs & Files as `dp_dba`. Server jobs grid is read from `DBA_DATAPUMP_JOBS`, includes jobs started by `expdp`/`impdp` and from other schemas.
- [ ] Start a long export (`t_big`). Auto-refresh (5 s) is on: state and percent update without clicking. Untick it: updates stop. Refresh button reloads once.
- [ ] Close the app during the running export, reopen, open Jobs & Files. The job is still listed and finishes on the server.
- [ ] Stop...: confirm. Job state becomes NOT RUNNING (master table kept). Resume...: optional parallel prompt, job goes back to EXECUTING and completes.
- [ ] Kill... on a running job: confirm. Job disappears from EXECUTING and its master table is dropped.
- [ ] Stop a job, then Clean Up...: confirm. The leftover master table is dropped and the job leaves the list. Clean Up is refused or disabled for a running job.
- [ ] Selection keeps its row across auto-refresh: select a job, wait 10 s (two refreshes), press Stop. It acts on the same job, not on the row now at that index. Repeat in the files grid: select a file, wait 10 s, press Download; it downloads that file.
- [ ] Non-DBA user (`dp_user`): jobs list falls back to `USER_DATAPUMP_JOBS` (own jobs only), no privilege error. The running-job check before Download still works (try Download while the job runs: refused).
- [ ] Files grid: Download... on a file of a still-running job is refused with "job is still running". After the job ends it works.
- [ ] Delete from Server...: confirm. The file is removed (`UTL_FILE.FREMOVE`) and shows as missing. Without `UTL_FILE` grant: hint naming the GRANT.
- [ ] Download by Name...: directory `DP_TEST_DIR`, a file name made outside the app (e.g. by `expdp` or an earlier Copy to Worksheet run). Several names separated by comma or space are all downloaded.
- [ ] View on a file, View Log on a job: viewer opens with the text. Close the viewer: temp files are removed (see Failure cases).
- [ ] Rename the connection in rsqldev: the local exports list still shows its rows. Delete the connection: rows stay, greyed. A second connection to the same database shows the same rows.

## Transfers

- [ ] Download a dump of several hundred MB. Progress shows bytes and rate; memory stays flat (no growth with file size). The local file appears as `<name>.part` while running and is renamed only when complete. Final size equals the server size.
- [ ] End of file, exact multiple: download a file whose size is an exact multiple of 1 MiB (e.g. create a 5 MiB one with `UTL_FILE` or upload one). Completes and the size equals the server size, with no extra empty read or hang.
- [ ] End of file, not a multiple: download a file whose size is not a multiple of 1 MiB (e.g. 5 MiB + 123 bytes). Completes with the exact server size.
- [ ] Resume: start a big download, press Cancel midway. The `.part` stays on disk. Download again: it continues from the `.part` size (progress starts near the previous point, not 0) and the final file is byte-identical (compare size and `shasum -a 256` with a fresh full download).
- [ ] Resume with a bad `.part`: make the `.part` larger than the server file (append junk). Download again: it restarts from 0 and ends with the exact server size.
- [ ] Download with the destination disk full (or read-only folder): a clear error; the `.part` is kept; no final file.
- [ ] Upload a small file (under 1 KiB), then a file of exactly 4 MiB, then 4 MiB + 100 bytes, then a file with size not a multiple of 4 MiB that ends in a small piece (e.g. 9 MiB + 1 byte). For each: the upload finishes, server size equals local size.
- [ ] Verify upload content: re-download each uploaded file and compare `shasum -a 256` with the original. All identical. (This confirms the directory, file name and mode binds as CLOB and the last chunk of up to 2000 bytes as RAW into `b BLOB := :4`; only a live Oracle can confirm it.)
- [ ] Cancel an upload midway. No crash; the partial server file is either removed or clearly reported; a retry succeeds and the result is byte-identical.
- [ ] Run a download while working in a worksheet on the same connection. The worksheet stays responsive (transfer uses its own session).
- [ ] Reconnect retry: during a SELECT-only call (jobs refresh, file exists check), kill the app's session on the server (`ALTER SYSTEM KILL SESSION '<sid>,<serial#>' IMMEDIATE` as DBA, or use an idle timeout). The next refresh succeeds transparently (reconnected and retried once), no error dialog.
- [ ] Reconnect without retry for writes: with autocommit on, kill the session and run an INSERT in a worksheet whose loss happens mid-call. The error is shown ONCE; the statement is not silently re-run (check the table: no duplicate row, at most one row).

## Failure cases

- [ ] Revoke `EXECUTE ON UTL_FILE` from `dp_user` (and from PUBLIC if granted). Upload and Delete from Server show a one-line hint naming the GRANT (PLS-00201 `UTL_FILE`), not a stack.
- [ ] Revoke `WRITE ON DIRECTORY` from `dp_user`. Export shows a one-line hint about the directory WRITE grant (ORA-39002/ORA-39087/ORA-29283). Revoke `READ` too: Download and View Log show a READ hint.
- [ ] Directory that does not exist: validation or a one-line "directory not found" hint (ORA-29283/ORA-39087).
- [ ] File that is not in the directory: Download shows a clear "file not found" (ORA-22285/ORA-29283), no `.part` is left behind.
- [ ] Non-DBA exports another schema or Full: hint names `DATAPUMP_EXP_FULL_DATABASE` (ORA-31631).
- [ ] Disk full on the server directory: the export job ends with an error; the log shows it; the UI shows the error state and the job can be Cleaned Up.
- [ ] Network drop during a download: a clear error, `.part` stays; reconnect and Download again resumes.
- [ ] Temp folder privacy: open View Log, and while the viewer is open run `ls -ld "$TMPDIR"/*` (macOS) and look for the folder the app made. It has a random name and mode `drwx------` (owner only). Close the viewer: the folder is removed. Repeat with the SQL-file open from a SQLFILE import and View on a file.
- [ ] Double-click Download twice fast, or press Stop twice: only one action runs (in-flight guard); no duplicate jobs or transfers.
- [ ] Job names are unique: two exports started in the same second do not collide (second gets an error or a different name, never silently attaches to the first).
