# rsqldev — Feature Catalog (spec from Oracle SQL Developer 26.2)

Clean-room spec. Built only from the declarative metadata shipped with SQL Developer 26.2: extension.xml, accelerators*.xml, dialogs/actions XML, navigator/viewer XML, connection XML, English `.properties` bundles, and jar listings. No class files were decompiled and no SQL was copied. §4.2 / §4.3 come from the Data Modeler table editor bundles, `CreateDialogArb` and the table action XML of 26.2. Reports (§7.1) are out of scope for the table / row editor work. Each row says what a feature does or shows, not how SQL Developer builds it.

**Priority tags:** `MVP` = needed for the first usable release. `P2` = next wave. `P3` = niche, or tied to a specific Oracle option.
**Shortcut notation:** `W` = Windows/Linux, `M` = macOS. If no `M` value is given, the Mac preset (`Default MacOS X`) has no override and uses the same key as Windows.
**Source tags:** (core) = `oracle.sqldeveloper.jar` / `dbtools-common`. (ide) = the JDeveloper IDE framework jars. (ws) = the worksheet jar. (ext:x) = the extension jar named x.

---

## 0. Scope summary

| Area | Count found in metadata | Notes |
|---|---|---|
| Extension jars | 59 | Oracle core, worksheet, DBA, reports, cart, migration (7 source DBs), Data Modeler, Data Miner, OLAP, HCS, REST/ORDS, spatial, RDF/PGQL, TimesTen, SSH, scheduler, security (redaction), cloud |
| Context-menu action sets | ~330 (object type × source) | Oracle core: ~45 object types. DBA navigator: ~60 node types |
| Object viewer definitions | ~300 | Oracle core: ~35 object types |
| Navigator object types | Oracle ~30 top folders + DBA ~55 nodes + 14 non-Oracle connection types | |
| SQL*Plus/SQLcl help topics | ~140 SET/SHOW subcommands + ~45 top-level commands + ~27 SQLcl extensions | |
| Preference pages | ~60 (core) + editor/debugger (extras) + module pages | |

---

## 1. Main menus and global toolbar

The IDE framework supplies the File/Edit/View/Navigate/Run/Team/Tools/Window/Help menus. SQL Developer adds items through menu hooks. Default keymap preset is "Default"; Mac uses "Default MacOS X". Other shipped presets (Classic, Emacs, Visual C++, Eclipse) are `P3`.

### 1.1 File
| Item | W | M | Pri |
|---|---|---|---|
| New… (gallery: connection, worksheet, object types) | — | — | MVP |
| Open… | Ctrl+O | Cmd+O | MVP |
| Save | Ctrl+S | Cmd+S | MVP |
| Save As… | — | Cmd+Shift+S | MVP |
| Save All | — | Cmd+Alt+S | MVP |
| Close | Ctrl+W / Ctrl+F4 | Cmd+W | MVP |
| Close All | Ctrl+Shift+F4 | Cmd+Alt+W | MVP |
| Recent files (File Navigator "Recent Files": clear, remove) | — | — | P2 |
| Print… / Page setup | Ctrl+P | Cmd+P / Cmd+Shift+P | P3 |
| Import / Export (IDE settings, connections) | — | — | P2 |
| Data Modeler submenu (Open/Close/Save design, Import DDL/ERwin/Designer/VAR/XMLA, Export DDL/CSV/AW, Print diagram to PDF/SVG/PNG/HTML) | DM keys, see §7.12 | | P3 |
| Exit | Alt+F4 | Cmd+Q | MVP |

### 1.2 Edit
| Item | W | M | Pri |
|---|---|---|---|
| Undo | Ctrl+Z / Alt+Backspace | Cmd+Z | MVP |
| Redo | Ctrl+Y / Ctrl+Shift+Z | Cmd+Shift+Z / Cmd+Y | MVP |
| Cut / Copy / Paste | Ctrl+X / Ctrl+C / Ctrl+V | Cmd+X/C/V | MVP |
| Extended Paste… (clipboard history) | Ctrl+Shift+V | Cmd+Alt+V | P2 |
| Copy Path | Ctrl+Shift+C | Cmd+Shift+C | P3 |
| Delete | Delete | Delete | MVP |
| Select All | Ctrl+A | Cmd+A | MVP |
| Find… | Ctrl+F | Cmd+F | MVP |
| Find Next / Previous | F3 / Shift+F3 (editor: Ctrl+F3 / Ctrl+Shift+F3) | Cmd+G / Cmd+Shift+G (editor: Cmd+E / Cmd+Shift+E) | MVP |
| Replace… | Ctrl+R | Cmd+R | MVP |
| Incremental search fwd/back | Ctrl+E / Ctrl+Shift+E | Cmd+D / Cmd+Shift+D | P2 |
| Find in Files (search source path) | Ctrl+Shift+F | Cmd+Alt+F | P2 |
| Duplicate selection | Ctrl+Shift+D | Ctrl+Shift+D | P2 |
| Toggle line comment | Ctrl+/ | Cmd+Alt+/ | MVP |
| Completion insight | Ctrl+Space | Ctrl+Space / Cmd+Alt+C | MVP |
| Parameter tooltip insight | Ctrl+Shift+Space | Cmd+Alt+P | P2 |
| Smart completion | Ctrl+Alt+Space | same | P2 |
| Expand / narrow selection | Ctrl+Shift+= / Ctrl+Shift+- | Ctrl+Cmd+= / Ctrl+Cmd+- | P3 |
| Multi-caret: select word / above / below | Alt+Shift+Space / Alt+Shift+PgUp / PgDn | — | P3 |
| Move line up / down | Ctrl+Shift+Up / Down | — | P2 |
| Join line / delete line / delete to EOL | Ctrl+J / Alt+Shift+D / Ctrl+Shift+Y | — / — / Cmd+Delete | P3 |
| Goto matching brace | Alt+[ or ] | Cmd+[ or ] | P2 |
| Fold: collapse/expand block, all | Ctrl+[ ]  / Ctrl+Shift+[ ] | Ctrl+[ ] / Ctrl+Cmd+[ ] | P2 |
| Toggle whitespace display | Ctrl+Shift+W | — | P3 |
| Preferences… (Mac: app menu) | Tools menu | Cmd+, | MVP |

### 1.3 View
| Item | Pri |
|---|---|
| Connections (Database Navigator) | MVP |
| Reports navigator | P2 |
| DBA navigator | P2 |
| Snippets | P2 |
| Find DB Object (also on main toolbar) | P2 |
| Files (File Navigator) | P2 |
| SSH navigator | P3 |
| REST Data Services navigator | P3 |
| Migration Projects navigator, Captured/Converted Models | P3 |
| Data Modeler: Browser, Navigator, Log, DDL Preview, DDL File Editor, Pending changes, Zoom, Search | P3 |
| Log window (Ctrl+Shift+L / Ctrl+Cmd+L) | MVP |
| Toolbars ▸ Database Navigator toggle | P2 |
| Task Progress, Recent Objects | P2 |
| Structure, Inspector (Ctrl+Shift+I / Ctrl+Cmd+I) | P3 |

### 1.4 Navigate
| Item | W | M | Pri |
|---|---|---|---|
| Back / Forward | Alt+Left / Alt+Right | Cmd+Alt+Left / Right | P2 |
| Go to Last Edit | Ctrl+Shift+Backspace | Cmd+Alt+Z | P2 |
| Go to Line… | Ctrl+G | Cmd+L | MVP |
| Next/prev editor tab | Alt+PgDn / Alt+PgUp | Cmd+PgDn / PgUp | MVP |
| Cycle editor frames | Ctrl+Tab / Ctrl+F6 (Shift = back) | Cmd+F3 (Shift = back) | P2 |
| Next / prev pane | F6 / Shift+F6 | Cmd+F6 / Cmd+Shift+F6 | P3 |
| Go to document 1..9; assign 1..9 | Alt+1..9; Alt+Shift+1..9 | Cmd+1..9; Ctrl+Cmd+1..9 | P3 |
| Show open-docs dropdown / recent files | Alt+0 / Ctrl+= | Cmd+0 / Cmd+= | P2 |
| Next / prev message | Alt+F8 / Alt+F7 | Cmd+F10 / Cmd+Shift+F10 | P3 |
| Toggle bookmark / next / prev | Ctrl+K / Ctrl+Q / Ctrl+Shift+Q | Cmd+K / Cmd+T / Cmd+Shift+T | P2 |
| Go to bookmark dialog | Ctrl+Shift+K | Cmd+Shift+K | P2 |
| Numbered bookmarks: toggle / goto | Ctrl+Shift+0..9 / Ctrl+0..9 | same | P3 |
| Open Declaration (PL/SQL identifier) | Ctrl+Click | Ctrl+Click | P2 |
| Go to Package Spec/Body | Ctrl+Shift+F12 | — | P2 |
| Quick search bar | Ctrl+` | Ctrl+` | P3 |

### 1.5 Run
| Item | W | Pri |
|---|---|---|
| Run PL/SQL (opens parameter dialog with generated anonymous block) | Ctrl+F10 | P2 |
| Debug PL/SQL | Ctrl+Shift+F10 | P3 |
| Compile / Compile for Debug | Ctrl+F8 / Ctrl+Shift+F8 | P2 |
| Toggle Breakpoint | F5 (PL/SQL editor) | P3 |
| Debugger controls (step over/into/out, resume, terminate, run to cursor) | IDE defaults | P3 |

### 1.6 Team (version control)
| Item | Pri |
|---|---|
| Git: clone, commit, push/pull, branches, pending changes, history, compare (ext: `oracle.jdeveloper.git`) | P3 |
| Subversion: checkout, update, commit, compare, properties, lock (also used by Data Modeler designs) | P3 |

### 1.7 Tools
| Item | Pri |
|---|---|
| SQL Worksheet (Alt+F10 on both platforms; also on main toolbar, defaults to the selected connection) | MVP |
| Database Copy… | P2 |
| Database Export… (export wizard) | P2 |
| Database Diff… | P2 |
| Monitor Sessions… (report-based session monitor) | P2 |
| Real Time SQL Monitor (Oracle only; needs Tuning Pack licence) | P3 |
| Data Modeler submenu (Domains, Types, RDBMS sites, mask templates, glossary, naming standards, design rules, compare) | P3 |
| Migration submenu (Quick Migrate, Create offline scripts, repository mgmt, Translation scratch editor, Application scan, Redshift cloud migration) | P3 |
| Data Miner, OLAP (time dim wizard), TimesTen | P3 |
| Features / Manage Features (enable/disable extensions) | P3 |
| External Tools | P3 |
| Preferences (Win/Linux) | MVP |

### 1.8 Window
| Item | Pri |
|---|---|
| Database ▸ Connections, Reports, DBA, Find DB Object, Snippets, Cart, Spatial map, SSH, Files, Migration projects, DBMS Output, OWA Output, SQL History, Task Progress, Recent Objects, Instance Viewer | MVP (Connections, DBMS Output, SQL History); rest P2/P3 |
| Reset Windows to factory layout | P2 |
| Switch Active View (Ctrl+Alt+V) | P3 |

### 1.9 Help
| Item | Pri |
|---|---|
| Contextual help (F1 / Shift+F1; Mac Cmd+/ or Cmd+Shift+/), Search, Table of contents, Start Page, Check for Updates, About | P2 (About: MVP) |

### 1.10 Main toolbar
New, Open, Save, Save All, Back/Forward, Undo/Redo, SQL Worksheet ▾ (connection picker), Find DB Object, Run/Debug. `MVP` for New/Open/Save/Worksheet.

---

## 2. Connections

### 2.1 Connection dialog (New / Select Database Connection)
Layout: a saved-connections list on the left; Name, Color, and a Database Type selector (Oracle plus the installed third-party types) on the right; tabbed details below; a status line; and the buttons **Help, Save, Clear, Test, Connect, Cancel**.

| Element | Detail | Pri |
|---|---|---|
| Connection Name | required; status shows "Please specify a Connection Name" | MVP |
| Color | colors the worksheet/editor border for this connection | P2 |
| Database Type | Oracle, or a third-party type (needs a JDBC driver, §2.5) | MVP |
| **User Info tab**: Authentication Type | Default / OS / Kerberos | MVP (Default), P2 (OS, Kerberos) |
| Username, Password, Save Password | hidden for OS auth | MVP |
| Role | Default, SYSDBA, SYSOPER, SYSBACKUP, SYSDG, SYSKM, SYSASM | MVP |
| **Proxy User tab** | Use DB Proxy Authentication; Proxy Type (User Name / Distinguished Name); Proxy Client; Proxy Password or DN | P3 |
| **Connection Type** | Basic, TNS, LDAP, Custom JDBC, Cloud Wallet, SSH, Local/Bequeath | see rows |
| Basic | Hostname, Port, Type = Service Name or SID | MVP |
| TNS | Network Alias (dropdown from tnsnames.ora) or Connect Identifier (free text); tnsnames dir set in prefs | MVP |
| Custom JDBC | Custom JDBC URL (free text) | MVP |
| LDAP | LDAP Server (host:port[:sslport]), Context, DB Service (browse list); optional admin user/password | P3 |
| Cloud Wallet | Configuration file (zip), Service (list read from wallet), Proxy (No / Custom host+port); OSS Classic: identity domain, service name, user/pass, service base URL | P2 |
| SSH (connection type) | Port Forward (pick from SSH navigator), SID or Service Name | P2 |
| Local / Bequeath | local instance, no listener | P3 |
| **SSH tab** (classic) | Use SSH; Host, Port, Username; Use Key File plus a key-file browser; validation messages | P2 |
| **Advanced tab** | Name/Value table of JDBC connection properties | P2 |
| Proxy (HTTP) | No Proxy / Use SQL Developer Proxy / Use Custom Proxy (host, port) | P3 |
| Use OCI (thick) | from prefs; OCI status test | P3 |
| Test | tries to connect; status "Success" or "Failure - <message>" | MVP |
| Save | saves the definition (password only if Save Password is ticked) | MVP |
| Connect | saves, connects, and expands the node; can open a worksheet automatically (pref) | MVP |
| Clear | resets the fields | MVP |
| Edit existing (Properties…) | same dialog, preloaded | MVP |
| Connection startup script | file run on connect (pref: "Filename for connection startup script") | P2 |
| Password expiry | prompts for a new password; "Reset Password…" action | P2 |

### 2.2 Connections tree behaviour
| Action | Pri |
|---|---|
| Connect / Disconnect / Reconnect | MVP |
| On disconnect with an open transaction: prompt "Connection X has uncommitted changes" — Commit / Rollback / Abort | MVP |
| Close all worksheets / reports on disconnect (prefs) | P2 |
| Properties… (edit), Delete, Rename Connection…, Duplicate (copy) | MVP |
| Open SQL Worksheet (for that connection) | MVP |
| Folders: Add to Folder (new/existing), Rename Folder, Remove Folder, Remove from Folder | P2 |
| Group Connections By Type (toolbar toggle) | P2 |
| Apply Filter… / Clear Filter (per connection: object name/type filter) | MVP |
| Refresh (F5), Collapse All | MVP |
| Disconnect Connections (bulk) | P2 |
| Create Local Connections (auto-create from local DB users) | P3 |
| Unshared worksheet / "Close Related Connections" | P2 |
| Manage Database (opens the DBA-style instance page) | P3 |
| Schema Browser (alternate view: schema + object-type dropdowns, filters) | P2 |
| Oracle-only connection actions: Gather Schema Statistics, Recompile Schema, Generate DB Doc, Remote Debug, XML DB protocol config, Liquibase ▸ Generate Schema, REST Services ▸ Enable/Drop | P3 |

### 2.3 Import / Export connections
| Feature | Pri |
|---|---|
| Export Connections wizard: pick connections → destination file (JSON; XML/properties are legacy) → password handling: **Encrypt all passwords with a key** (key + verify) or **Strip passwords** → summary | P2 |
| Import Connections wizard: source file → pick connections → duplicates **Replace / Rename** → decryption key → summary; detects old format | P2 |
| Formats accepted on import: JSON, XML, properties (legacy) | P2 |
| Connection storage: per-user config dir, passwords encrypted at rest | MVP |

### 2.4 Cloud
| Feature | Pri |
|---|---|
| New Cloud Connection… (OCI/Autonomous); Cloud Connect/Disconnect/Reconnect | P3 |
| Cloud Deployments node (status, restart, clear/download logs, delete) | P3 |

### 2.5 Driver management (Preferences ▸ Database ▸ Third Party JDBC Drivers)
| Feature | Pri |
|---|---|
| List of driver jar paths (Add/Remove); new DB types appear in the connection dialog | MVP (rsqldev: native driver registry per DB type) |
| Built-in drivers: Oracle thin (OCI optional), MongoDB JDBC bundled | MVP |
| Third-party types: MySQL, SQL Server, Sybase, DB2 (LUW), DB2 for i, Teradata, PostgreSQL, Amazon Redshift, MS Access, Hive, Informix, MongoDB, TimesTen, generic JDBC | MVP: PostgreSQL, MySQL, SQL Server; P2 rest |

---

## 3. Object browser (Connections tree) — node types

### 3.1 Oracle (connType Oracle)
Folder order follows the navigator weights, roughly as listed. Child nodes are in ().

| Folder | Children / notes | Pri |
|---|---|---|
| Tables (filtered) | table → Columns, Partitions; quick filter | MVP |
| Views | view → columns | MVP |
| Editioning Views | | P3 |
| JSON Relational Duality Views | → columns (23ai+) | P3 |
| Indexes | | MVP |
| Packages | package → Body, Members (proc/func list); Order Members By (source/name/type) | MVP |
| Procedures | → args | MVP |
| Functions | → args | MVP |
| Operators | | P3 |
| Queues | | P3 |
| Queues Tables | | P3 |
| Triggers | | MVP |
| Crossedition Triggers | | P3 |
| Types | → Body, attributes | P2 |
| Sequences | | MVP |
| Materialized Views | → columns | P2 |
| Materialized View Logs | | P2 |
| Synonyms | | MVP |
| Public Synonyms | | P2 |
| Database Links | | P2 |
| Public Database Links | | P2 |
| Directories | | P2 |
| Editions | nested editions | P3 |
| Application Express | app → pages | P3 |
| Java | classes/sources | P3 |
| XML Schemas | | P3 |
| XML DB Repository | nested folders | P3 |
| Scheduler | Chains, Credentials, DB Destinations, DB Destination Groups, External Destination Groups, File Watchers, Jobs, Programs, Schedules (DBA-only: Global Attributes, Job Classes, External Destinations, Windows, Window Groups) | P2 |
| DBMS Jobs (legacy) | | P3 |
| Recycle Bin | | P2 |
| Other Users | user → the full folder set for that schema | MVP |
| Analytic Views / Attribute Dimensions / Hierarchies (HCS) | measures, calc measures, levels | P3 |
| OLAP: Analytic Workspaces, Cubes, Cube Dimensions, build jobs | | P3 |
| RDF Semantic Graph (networks, models, views, rulebases…) | | P3 |
| Property Graph: PGQL Property Graphs, SQL Property Graphs | | P3 |
| Spatial (spatial tables) | | P3 |
| REST Data Services: Modules → Templates → Handlers → Parameters; Privileges; Roles | | P3 |
| Cloud Deployments | | P3 |

### 3.2 DBA navigator (connType "Oracle DBA", separate panel; connections added explicitly)
| Group | Nodes | Pri |
|---|---|---|
| Container Database | PDBs, Application Roots/PDBs/Applications | P3 |
| Database Configuration | Initialization Parameters, Automatic Undo Management, Current DB Properties, Restore Points, Services, Feature Usage | P3 |
| Database Status | Status, Instance Viewer, Alert Log | P2 |
| Data Guard | Console, Databases, Local Values | P3 |
| Data Pump | Export Jobs, Import Jobs | P3 |
| Performance | Snapshots, Baselines, Baseline Templates, ADDM, ASH Report Viewer, AWR (CDB + PDB variants) | P3 |
| RMAN Backup/Recovery | Backup Jobs, Backup Sets, Image Copies, RMAN Settings, Scheduled RMAN Actions | P3 |
| Resource Manager | Consumer Groups, Mappings, Plans, Settings, Statistics; Container RM Plans/Settings/Stats | P3 |
| Scheduler (DBA) | as in §3.1 | P3 |
| Security | Users (granted roles, sys privs), Roles, Profiles, Audit Settings | P2 |
| Sharding | Sharded DB Viewer, Shard Spaces, Regions, Shard Groups, Shards, Tablespace Sets, Directors | P3 |
| Storage | Tablespaces, Control Files, Temp Tablespace Groups, Datafiles, Rollback Segments, Redo Log Groups (members), Archive Logs | P2 |
| Real Time SQL Monitor, Tuning Memory, Metrics (sample/instance/table/monitor viewers) | | P3 |

### 3.3 Non-Oracle node types
| DB | Folders | Pri |
|---|---|---|
| MySQL | Tables, Views (cols), Indexes, Procedures, Functions, Triggers, Other Users | MVP |
| PostgreSQL | Databases, Tables (cols), Views (cols), Indexes, Procedures, Functions, Triggers, Other Users | MVP |
| SQL Server | Databases, Tables, Views, Indexes, Procedures, Functions, Triggers, Synonyms, Defaults, Rules, User-Defined Data Types, Users, Logins, Other Users | MVP |
| Sybase ASE | Databases, Tables, System Tables, Views, Indexes, Procedures, Functions, Triggers, Defaults, Rules, Segments, Groups, Users, User Defined Datatypes, Other Users | P2 |
| DB2 (LUW) | Tables, Views, Aliases, Indexes, Procedures, Functions, Triggers, UDTs, Other Users | P2 |
| DB2 for i | as DB2 + Sequences | P3 |
| Teradata | Tables, Views, Indexes, Macros, Procedures, Functions, Triggers, Other Users | P2 |
| Amazon Redshift | Databases, Tables, Views, Indexes, Procedures, Functions, Triggers, Other Users | P2 |
| MS Access | Tables, Queries, Forms, Reports, Macros, Modules, Schema | P3 |
| Hive | Tables, Views, Indexes, Other Users | P3 |
| Informix | Tables, Indexes | P3 |
| MongoDB | Collections | P3 |
| Generic JDBC | Tables | P2 |
| TimesTen | Tables, Views, Indexes, Packages, Procedures, Functions, Synonyms, Public Synonyms, MViews, MView Logs, Sequences, Profiles, Cache Groups, Replication Schemes, Other Users | P3 |
| Non-Oracle connection menu | Rename Connection, Open SQL Worksheet; Copy To Oracle (table/collection, or whole Access DB); SQL Server/Sybase: Select Default Database | MVP (worksheet), P2 (copy) |

---

## 4. Per object type: context menu and object viewer tabs (Oracle)

Every object node also has the generic actions **Open** (object viewer), **Edit…** (Create/Edit dialog), **Refresh**, **Quick DDL ▸ Save to File / Worksheet / Clipboard**, **Export…** (wizard), **Add To Cart…**, **Copy** (name), and **Popup Describe** (Shift+F4; Mac Alt+Shift+F4). Folder nodes have **New <type>…**, **Apply Filter…**, **Clear Filter**, and **Refresh**. Many types also have **Generate Changelog (Liquibase)** (`P3`).

Generic object viewer features: a **Freeze/Pin** view toggle (Ctrl+Shift+P / Cmd+Shift+P), refresh, an Actions… menu (same as the context menu), and per-tab quick filtering. The **SQL** tab shows generated DDL, with options set in prefs (§7.20).

| Object type | Context-menu actions | Viewer tabs | Pri |
|---|---|---|---|
| TABLE | Edit; Import Data…; **Table▸** Rename, Copy, Drop, Truncate, Lock, Comment, Parallel/No Parallel, Logging, Count Rows, Generate Table API; **Column▸** Add, Rename, Drop, Comment, Normalize; **Constraint▸** Add PK/FK/Unique/Check, Enable/Disable all, Enable/Disable/Rename single, Enable/Disable related FKs, Drop; **Index▸** Create, Drop, Rebuild; **Privileges▸** Grant/Revoke; **Statistics▸** Gather, Validate Structure; **Storage▸** Shrink, Row Movement, Compress/No Compress, Move, Move Tablespace, Cache/No Cache; **Trigger▸** Create, Create (PK from Sequence), Enable/Disable all/single, Drop; Redaction▸ (security); Spatial▸; REST▸ Enable REST Service | Columns (name, type, nullable, default, column id, comment, PK flag); **Data** (editable grid); Model (diagram); Constraints (+columns); Grants; Statistics (+column stats); Triggers; Flashback (Data, Undo SQL); Dependencies (+References); Details (all table attributes); Partitions; Indexes; SQL | MVP: Columns, Data, Constraints, Indexes, SQL, Details, Drop/Rename/Truncate/Comment/Count Rows. P2: grants, stats, triggers, partitions, storage. P3: flashback, redaction, spatial, table API |
| COLUMN | Rename, Drop, Comment, Encrypt, Decrypt, Normalize; Redaction policies; Spatial metadata/index | — | P2 |
| VIEW | Edit; Rename, Drop, Compile; Grant/Revoke; Redaction▸; Spatial▸; Enable REST | Columns, Data, Grants, Dependencies, Details, Triggers, SQL, Errors | MVP: Columns, Data, SQL, Compile, Drop |
| VIEW folder | Compile Invalid, Create (simple) | — | P2 |
| DUALITY VIEW | Rename, Drop, Compile, Grant/Revoke | Columns, Data, Grants, Dependencies, Details, Triggers, SQL, Errors | P3 |
| INDEX | Drop, Rebuild, Rename, Make Unusable, Coalesce, Compute Statistics, Rebuild/Drop Partition | Details, Columns, Statistics, Partitions, SQL | MVP: Details, Columns, SQL, Drop/Rebuild |
| SEQUENCE | Edit, Drop | Details, Dependencies, SQL | MVP |
| SYNONYM / PUBLIC SYNONYM | Rename, Drop; folder: New (Public) Synonym | Details, SQL | MVP |
| PACKAGE | Edit (opens PL/SQL editor), Edit Body, Create Body, Run, Compile, Compile for Debug, Debug, Drop Package, Grant/Revoke, Save Package Spec and Body, Order Members By, Enable REST | Code, Grants, Dependencies, References, Errors, Details | MVP: Code, Errors, Edit, Compile, Drop |
| PACKAGE BODY | Drop Body, Order Members By, Compile | same | MVP |
| PROCEDURE / FUNCTION | Edit, Run, Compile, Compile for Debug, Debug, Drop, Grant/Revoke, Compile Dependants, Enable REST | Code, Grants, Dependencies, References, Errors, Details | MVP |
| PL/SQL folder (pkg/proc/func/trigger/type) | Compile Invalid…, Compile All… | list grid | P2 |
| TRIGGER | Edit, Rename, Drop, Enable, Disable, Compile | Code, Grants, Dependencies, References, Errors, Details | MVP |
| TYPE / TYPE BODY | Edit, Edit Body, Create Body, Drop, Grant/Revoke, Order Members By | Code, Dependencies, References, Errors, Grants, Details | P2 |
| MATERIALIZED VIEW | Drop, Grant/Revoke; Caching on/off; Compression on/off; Query Rewrite on/off; Parallelism; Logging; Refresh Option▸ Change Method/Type, Force Refresh Now; Other▸ Row Movement, Shrink, Compile, Consider Fresh, Rebuild; folder: Create (simple) | Columns, Data, Indexes, Grants, Dependencies, Details, SQL | P2 |
| MATERIALIZED VIEW LOG | Drop | Columns, Dependencies, Details, SQL | P2 |
| DATABASE LINK / PUBLIC | Test Database Link, Drop | Details, SQL | P2 |
| DIRECTORY | Create Directory (folder), Grant, Revoke, Drop | Details, SQL | P2 |
| QUEUE | Start/Stop Enqueue, Start/Stop Dequeue | Details, SQL, Schedules, Subscribers | P3 |
| QUEUE TABLE | — | Details, Data, Queues, Schedules, SQL | P3 |
| JAVA | Load Java (folder), Drop Class | Details, Source, Policies | P3 |
| DBMS JOBS | Submit (folder); Run, Remove, Force Broken/unBroken, Change, Instance, Interval, Next Date, What | Details, Your/All/DBA Jobs | P3 |
| RECYCLE BIN | Purge, Flashback to Before Drop | Details | P2 |
| EDITION | Set Current, Create, Drop | Edition | P3 |
| XML SCHEMA / XDB folder | Drop schema; Create Subfolder, Drop Folder | XDB Folder Info | P3 |
| USER (Other Users) | Create User (folder), Edit User, Drop User | Details, SQL | P2 |
| APEX app / page | Import Application, Deploy Application | App: Application, Pages, LOVs, Lists, Templates, Breadcrumbs, Tabs, Nav Bar, Items, Computations, Processes, Authorization, Install/Upgrade/Deinstall, SQL. Page: Page, Items, Regions, Buttons, Computations, Processes, Validations, Branches, SQL | P3 |
| SCHEDULER JOB | New Job / New Job (Wizard); Edit, Enable/Disable, Run Job, Drop, Create Like | Details, Notifications, Job Arguments, Properties, Dependencies, Run Log (run details, script output) | P2 |
| SCHEDULER program/schedule/chain/credential/destinations/file watcher/class/window(group) | Edit, Drop, Enable/Disable, Create Like; Chain: Run Chain; Scheduler Design Editor | Details (+Arguments / Steps, Rules / Members), Dependencies | P3 |
| REST module/template/handler/privilege/role | New Module, Add Template, Add Handler GET/POST/PUT/DELETE, Edit, Rename, Delete, Validate, REST Definition ▸ Save to File/Worksheet/Clipboard | Module: General, Templates, Privilege, Origins Allowed. Template: General, Handlers. Privilege: General, Roles, Protected Modules/Resources | P3 |

### 4.1 DBA navigator actions (all `P3` unless noted)
| Node | Actions | Viewer tabs |
|---|---|---|
| USER | Create New, Edit, Create Like, Drop, Expire Password, Lock/Unlock, Grant Role, Grant System Privilege; revoke on children | General, Roles, Sys Privs, Object Privs, Quotas, Consumer Group Privs, Proxy Users, Proxied Users, SQL (`P2`) |
| ROLE | Create, Edit, Create Like, Drop, Grant Role/Sys Priv | General, Roles, Sys Privs, Object Privs, Consumer Group Privs, User/Role Grantees, SQL |
| PROFILE | Create, Edit, Create Like, Drop | General, Database Services, Password, SQL |
| TABLESPACE | Create, Edit, Add Datafile, Change Storage Mgmt / Read State / Online-Offline, Drop | Details, Datafiles, Free Space, Objects, Usage Chart, SQL (`P2`) |
| DATAFILE | Create, Edit, Create Like, Online/Offline, Delete | General, SQL |
| REDO LOG GROUP / member | Create, Force Checkpoint, Switch Logfile, Add Member, Clear Logfile, Delete | Members, SQL |
| ROLLBACK SEGMENT | Create, Edit, Online/Offline, Shrink, Delete | Details, SQL |
| CONTROL FILE | Backup to Trace | General, Advanced, Record Section |
| PDB / App root / App PDB | Create PDB / App Container, Clone (also to Cloud), Drop, Plug In, Unplug, Relocate, Refresh, Modify State, Create Startup Trigger, Sync Application | General, Dynamic, Data Files |
| RMAN | Create Backup Wizard; Disk/Tape/custom backups; Restore/Recover; Catalog Recovery Area; Crosscheck; Delete Obsolete/Expired; Validate; device/policy settings; switch archivelog / flashback | Summary, Datafiles, Control Files, SPFile, Archive Log, Backup Sets/Pieces, Image Copies, Job Output |
| AWR / ADDM / Baselines / Snapshots | Run ADDM, Create Snapshot/Baseline/Template, Edit Settings, Delete | Findings, ADDM Report, AWR/ASH Report Viewer |
| Resource Manager | Create/Edit/Activate/Deactivate plans, directives, group mappings | Details, directives, statistic charts |
| Audit settings | Edit Audited Privileges/Objects/Statements | Configuration, Default Options, trails |
| Services | Create, Edit, Start, Stop, Delete | Details |
| Status / Instance Viewer | — | Database Status, Host, Home, Listener; live instance dashboard (`P2`) |

### 4.2 Create / Edit Table dialog (ext:datamodeler, core `CreateDialogArb`)

Same dialog for Tables ▸ New Table… and Table ▸ Edit…; title "Create Table" / "Edit Table". Panels (left list): Columns, Primary Key, Unique Constraints, Indexes, Foreign Keys, Table Constraints, Comments, Storage Options, DDL, Summary. Every change is shown as DDL before it runs.

| Panel | Fields | Priority |
|---|---|---|
| Header | Schema, Name, Temporary Table (Session / Transaction), Comment | MVP |
| Columns | grid PK, Name, Data Type, Size, Scale, Not Null, Default, Comment; Add (Shift = insert before, Ctrl = after), Remove, Move Up / Down; **Column Properties**: Data type Simple / Complex, Type, Size / Precision, Scale, Units (BYTE / CHAR), Default, Default on NULL, Cannot be NULL, Virtual (Expression), Identity / Auto Increment (Generate ALWAYS / BY DEFAULT / BY DEFAULT ON NULL, Start With, Increment By, Min Value, Max Value, Cycle, Cache, Order) | MVP (identity, virtual: P2) |
| Primary Key | Name, Enabled, Available ↔ Selected columns with Move Up / Down | MVP |
| Unique Constraints | list (Add / Remove); Name, Enabled, Available ↔ Selected columns | MVP |
| Foreign Keys | list; Name, Enabled, Referenced Schema, Referenced Table, Referenced Constraint, Associations (local column ↔ referenced column), On Delete (No Action / Cascade / Set Null) | MVP |
| Table Constraints (check) | list; Name, Condition, Enabled | MVP |
| Indexes | list (Add / Remove); Name, Unique, Index Expressions (Add / Remove / Move Up / Down, Order ASC / DESC) | MVP |
| Storage Options | Tablespace, Pct Free, Pct Used, Ini Trans, Max Trans, extents, Buffer Pool, Logging | P2 |
| DDL / Summary | the exact statements; OK runs them | MVP |

Validation messages: a table must include at least one column; duplicate column name; size / scale out of range for the type; "Reducing the size of a column is not supported"; "An object named X already exists".

### 4.3 Table actions (`META-INF/actions/oracle/table*.xml`)

Each action opens a small form, then a confirmation that shows the exact SQL.

| Submenu | Actions | Priority |
|---|---|---|
| Table | Rename, Copy (new owner, new name, Include Data), Drop (Cascade Constraints, Purge), Truncate (Storage), Lock (Lock Mode, No Wait), Comment, Parallel (Degree) / No Parallel, Logging, Count Rows, Generate Table API | MVP (Generate Table API: P3) |
| Column | Add (name, data type, precision, scale), Rename, Drop, Comment, Normalize | MVP (Normalize: P3) |
| Constraint | Add Primary Key / Foreign Key / Unique / Check, Enable all / Disable all, Enable / Disable Single (VALIDATE / NOVALIDATE), Enable / Disable Related Foreign Keys, Rename Single, Drop | MVP |
| Index | Create Index, Drop, Rebuild (Online) | MVP |
| Trigger | Create, Create (PK from Sequence) (trigger name, sequence name, column), Enable All / Disable All, Enable / Disable Single, Drop | MVP |

---

## 5. SQL Worksheet

### 5.1 Worksheet actions and shortcuts
| Action | W | M | Pri |
|---|---|---|---|
| Run Statement (statement at cursor/selection → Query Result grid) | Ctrl+Enter, F9 | Ctrl+Enter, F9 | MVP |
| Run Script (all/selection → Script Output, SQL*Plus semantics) | F5 | F5 | MVP |
| Run Script Output as a Script | Ctrl+F5 | — | P3 |
| Autotrace… | F6 | F6 | P2 |
| Explain Plan… (dropdown: PLAN_TABLE, DBMS_XPLAN, V$SQL_PLAN child #0/#1/#2) | F10 | F10 | MVP |
| Search execution plan | Ctrl+M | — | P3 |
| SQL Tuning Advisor… | Ctrl+F12 | Cmd+F12 | P3 |
| Commit | F11 | F11 | MVP |
| Rollback | F12 | F12 | MVP |
| Cancel running statement (toolbar stop; "Cancel Query?") | — | — | MVP |
| Clear worksheet | Ctrl+D | Cmd+D | MVP |
| SQL History window | F8 | — | MVP |
| History: replace with previous/next | Ctrl+Up / Ctrl+Down | Cmd+Up / Cmd+Down | MVP |
| History: append previous/next | Ctrl+Shift+Up / Down | Cmd+Shift+Up / Down | P2 |
| Unshared SQL Worksheet (own session) | Ctrl+Shift+N | Cmd+Shift+N | MVP |
| New worksheet | Alt+F10 | Alt+F10 | MVP |
| Format (whole doc / selection) | Ctrl+F7 | Ctrl+F7 | MVP |
| Format to Clipboard; Embed/Expose (SQL in code strings) | — / Ctrl+Shift+F7 | | P2 |
| To Upper/Lower/InitCap | Alt+' | — | P2 |
| Toggle multi-line comment | Alt+Shift+C | — | P2 |
| Refactor: Toggle ANSI/Oracle joins, Extract, Rename local var, Inline, Obfuscate, Surround with FOR/WHILE/IF/block | — | | P3 |
| Popup Describe (object under cursor) | Shift+F4 | Alt+Shift+F4 | P2 |
| Open Declaration | Ctrl+Click | | P2 |
| Code completion insight (tables/columns/keywords/aliases) | Ctrl+Space | | MVP |
| Code Templates insert | (pref-defined abbreviations) | | P2 |
| Save Snippet / drag snippet into editor | — | | P2 |
| Connection selector dropdown (per worksheet) | — | | MVP |
| Toggle line numbers, bookmarks (gutter menu) | | | MVP (line numbers) |
| Query Builder tab (visual query designer) | | | P3 |
| Rename Tab | | | P2 |
| Print (Ctrl+P), Save (Ctrl+S), worksheet saved as .sql file | | | MVP |
| Drag & drop table from tree → SELECT/INSERT/UPDATE/DELETE/JOIN text (pref: prompt or default) | | | P2 |
| Bind variables: "Enter Bind Values" dialog for `:x`; saved to disk on exit (pref) | | | MVP |
| Substitution variables `&x`/`&&x`: "Enter Substitution Variable" prompt | | | MVP |
| Code advice / SQL Patroller (lint), Tag SQL | | | P3 |
| Statement status bar: elapsed time "{n} rows in {t} seconds", caret position, insert mode | | | MVP |
| Results grid pin/freeze (Ctrl+I in result context), Show query results in new tabs (pref) | | | P2 |

### 5.2 Panes below the editor
| Pane | Contents and behaviour | Pri |
|---|---|---|
| Query Result | grid (§6); multiple tabs when "results in new tabs" is on; elapsed time; "SQL" button shows the executed query (Ctrl+Q) | MVP |
| Script Output | text log of a script run: feedback lines ("n rows affected", "Command completed successfully"), errors with line/column, DBMS output if SERVEROUTPUT is on; toolbar Clear / Save / Print; max lines pref | MVP |
| Explain Plan | tree table: Operation, Object Name, Options, Cost, Cardinality, Bytes, Predicates (access/filter), Projection, Partition start/stop/id, Other XML; expand/collapse; hotspot highlight; search; compare plans; export HTML. Columns chosen in prefs | MVP |
| Autotrace | plan with runtime columns (Last Output Rows, Last CR Buffer Gets, Last Elapsed Time, Last Starts) + V$MYSTAT session statistics; skip zero/NULL options; needs privileges (shows a message if they are missing) | P2 |
| DBMS Output (dockable) | per-connection tabs: enable (New, Ctrl+N), Clear (Ctrl+D), Save (Ctrl+S), Print (Ctrl+P); buffer size (or unlimited); poll frequency (max 15 s) | MVP |
| OWA Output | same model for the HTP buffer; enable per connection | P3 |
| SQL Tuning Advisor | Overview, Findings (statistics/index/profile/restructure), Recommendations, Rationale, Explain plans, generated script | P3 |
| SQL History (window) | grid: SQL, Connection, Type (statement/script), Executed timestamp, Duration; filter; Append / Replace into worksheet; Clear history (confirm); limit in prefs; persisted across sessions | MVP |

### 5.3 Script runner — supported SQL*Plus / SQLcl commands
Top-level: `/`, `@`, `@@`, `ACCEPT`, `APPEND`, `ARCHIVE LOG`, `BREAK`, `BTITLE`, `CHANGE`, `CLEAR`, `COLUMN`, `COMPUTE`, `CONNECT`, `DEFINE`, `DEL`, `DESCRIBE`, `DISCONNECT`, `EXECUTE`, `EXIT`/`QUIT`, `GET`, `HOST`, `INPUT`, `LIST`, `PASSWORD`, `PAUSE`, `PRINT`, `PROMPT`, `REMARK`, `RUN`, `SAVE`, `SET`, `SHOW`, `SHUTDOWN`, `SPOOL`, `START`, `STARTUP`, `TIMING`, `TTITLE`, `UNDEFINE`, `VARIABLE`, `WHENEVER SQLERROR/OSERROR`, `XQUERY`.

| Group | Commands | Pri |
|---|---|---|
| Core script execution | `/`, `@`, `@@`, `START`, `EXIT`, `PROMPT`, `REMARK`, `DEFINE`/`UNDEFINE`, `VARIABLE`/`PRINT`, `EXECUTE`, `WHENEVER`, `SPOOL`, `CONNECT`/`DISCONNECT`, `DESCRIBE`, `SET SERVEROUTPUT`, `SET DEFINE`, `SET ECHO`, `SET FEEDBACK`, `SET TIMING`, `SET VERIFY`, `SET TERMOUT`, `SET HEADING`, `SET PAGESIZE`, `SET LINESIZE`, `SET NULL`, `SET AUTOCOMMIT`, `SHOW ERRORS`, `SHOW USER` | MVP |
| Formatting | `COLUMN`, `BREAK`, `COMPUTE`, `TTITLE`/`BTITLE`, `SET COLSEP/NUMFORMAT/NUMWIDTH/LONG/WRAP/TRIMSPOOL/TRIMOUT/TAB/NEWPAGE/HEADSEP/RECSEPCHAR/UNDERLINE` | P2 |
| `SET SQLFORMAT` | ansiconsole, csv, json, insert, loader, fixed, html, xml, delimited, text (output format for queries in scripts) | P2 |
| Other SET | APPINFO, ARRAYSIZE, AUTOPRINT, AUTORECOVERY, AUTOTRACE, BLOCKTERMINATOR, CMDSEP, CONCAT, COPYCOMMIT, ENCODING, ERRORLOGGING, ESCAPE, EXITCOMMIT, FLUSH, HISTORY, MAXROWS, PAUSE, SECURELITERALS, SQLBLANKLINES, SQLCASE, SQLPROMPT, SQLTERMINATOR, SUFFIX, TIME, XQUERY, DDL (DBMS_METADATA transform options), CLOUDCONFIG, WALLET, SOCKSPROXY, STATUSBAR, HIGHLIGHTING, DATAPUMP, LOAD/LOADFORMAT, CODESCAN | P3 |
| SHOW | ALL, CONNECTION, CON_ID/CON_NAME, EDITION, ENCODING, ERRORS, INSTANCE, JDBC, NLS, PARAMETERS, PDBS, RECYCLEBIN, RELEASE, SGA, SPPARAMETERS, SQLCODE, TNS, USER, VERSION, SPOOL | P2 |
| SQLcl extensions | `ALIAS`, `APEX`, `ARBORI`, `BRIDGE`, `CD`, `CODESCAN`, `CTAS`, `DDL`, `EDIT`, `FIND`, `FORMAT`, `HISTORY`, `INFORMATION`/`INFO`, `KEYMAP`, `LOAD`, `NET`, `OERR`, `REST`, `SCRIPT` (JS), `SODA`, `SSHTUNNEL`, `STORE`, `TNSPING`, `TOSUB`, `UNLOAD`, `WHICH`, Liquibase (`lb`), Data Pump (`dp`) | P3 (DDL, INFO, OERR: P2) |
| Line-editor buffer | `APPEND`, `CHANGE`, `DEL`, `INPUT`, `LIST`, `GET`, `SAVE`, `RUN` | P3 |
| DBA | `STARTUP`, `SHUTDOWN`, `ARCHIVE LOG`, `PASSWORD` | P3 |
| Script runner prefs | max rows printed in a script; max lines in Script Output; default script path; SQLPATH; re-initialise on EXIT | P2 |

### 5.4 PL/SQL editor (code editor for stored units)
| Feature | Pri |
|---|---|
| Opens from Edit/Open; toolbar Run, Debug, Compile ▾ (Compile / Compile for Debug), Switch to Read Only (Ctrl+Shift+F11), Pin editor (Ctrl+Shift+F1), Go to spec/body | MVP (open/edit/compile) |
| Compiler log ("Compiler - Log" pane) with error list; click jumps to line/col | MVP |
| PL/SQL compiler prefs: warning levels (per category), optimisation level, PL/Scope identifiers | P3 |
| Syntax highlighting (PL/SQL colors + custom rules), outline/structure, folding, completion | MVP (highlighting/folding) |

---

## 6. Result grid (Query Result and Data tab)

| Feature | Details | Pri |
|---|---|---|
| Incremental fetch | first batch = SQL Array Fetch Size (50–200 in prefs); fetches more on scroll ("Fetching next n rows"); status "Fetched Rows: n" / "All Rows Fetched: n"; option "Fetch all rows" | MVP |
| Count Rows… | context menu; result shows "n Rows" or "n Rows (with filter applied)" | MVP |
| Sort | click a header for quick sort; Sort… dialog: multiple columns, Asc/Desc, Nulls First/Last; Sort History | MVP |
| Filter | Data tab "Filter:" box with a WHERE clause; column-header filter (Filter Column… picks distinct values); Remove All Filters; (Ctrl+Alt+F filter data) | MVP |
| Find / Highlight… | search the grid: whole word, starts with, ignore case, wrap, all columns; highlight row with a color; persisted highlights | P2 |
| Single Record View… | form view with First / Previous / Next / Last | MVP |
| Columns… | reorder, hide, show columns (Manage Columns dialog) | P2 |
| Auto-fit | all/selected columns, on header / on data / best fit / fit screen | P2 |
| Copy | cells as tab-separated; Copy with Headers (Ctrl+Shift+C); Copy Selected Column Header(s) | MVP |
| Paste into grid | multi-cell paste in edit mode (reports cells it could not paste) | P2 |
| Export… | wizard: format, header on/off, encoding, delimiter, enclosures, line terminator, output = clipboard/file/separate files/worksheet; whole result or selected rows | MVP (CSV, JSON, INSERT), P2 (Delimited, Fixed, HTML, Text, XLSX, XML, SQL*Loader, PDF, XLS, Merge/TTBulkCopy) |
| Persisted grid settings | column widths/order/sort per table; Delete Persisted Settings (Ctrl+Shift+G) | P2 |
| Freeze / pin result tab | keep the result when the next statement runs | P2 |
| Show SQL | display the query behind the grid (Ctrl+Q) | P2 |
| NULL display | "Display Null Value As" text + optional background color | MVP |
| Zebra / checkerboard rows | pref | P2 |
| Struct / XML / LOB display | struct values inline (pref); XML in grid (pref); LOB/large text cell → popup editor (…); BLOB: view as image/text, load/save to file, external editor | P2 |
| Cell popup editor | Alt+Enter: view/edit long text, date picker | MVP |
| Data editing (Data tab and editable queries) | Insert Row (Ctrl+I / Cmd+I), Duplicate Row, Delete Selected Row(s) (Ctrl+D / Cmd+D), edit cells inline; changed rows marked; Commit Changes (F11), Rollback Changes (F12), Refresh (Ctrl+R / Cmd+R); "Post edits on row change" and autocommit prefs; errors reported per row ("Row n: message") in the Messages - Log; Rollback asks "Do you want to rollback changes ?"; context menu Insert Row, Duplicate Row, Delete Selected Row(s), Paste (reports values it cannot paste), Single Record View, Count Rows…, Sort… (multi-column), Filter Column…, Remove All Filters | MVP |
| Date/number formatting | follows NLS prefs (date/timestamp formats) | MVP |
| Charts / graphical view in reports | | P3 |

---

## 7. Other modules

### 7.1 Reports (Reports navigator)
| Feature | Pri |
|---|---|
| Navigator: Data Dictionary Reports (shipped, read-only), User Defined Reports (folders, New Folder/New Report/Edit/Copy/Cut/Paste/Delete, Open Report file / Save As XML, HTML export), Shared Reports (via UDE), Data Modeler / ORDS / Scheduler / DBA reports | P2 |
| Run report: choose connection, bind prompts; output as grid, chart, gauge, code, script, PL/SQL DBMS output, HTML; master-detail child reports; drill-down; Back; open in new tab; auto-refresh | P2 |
| Shipped report groups: About Your Database (NLS params, version banner); All Objects (all, distribution, invalid, counts by type, collection types, dependencies, public synonyms/db links); Application Express; ASH and AWR; Data Dictionary (views, view columns); Database Administration ▸ Parameters, Storage (tablespaces, segments, free space, quotas, datafiles), Sessions (by status/OS user/module/user, active/inactive/background, CPU/time), Cursors, All Tables (columns, comments, constraints, indexes, triggers, statistics, storage, organisation, QA: no PK, no index, unindexed FKs), Top SQL (CPU, disk reads, buffer gets, executions, waits), Users, Alert Log, Locks (blocking), Memory (SGA), Waits and Events; PL/SQL (arguments, line counts, Search Source Code); Security (grants, public grants, auditing, encryption, policies, redaction); Streams; Table; XML; Scheduler | P2 (Search Source Code, Invalid Objects, Sessions, Locks, Top SQL = early P2) |
| Monitor Sessions (Tools): live session list + details, kill session | P2 |

### 7.2 DBA panel
See §3.2 and §4.1. Status, Users/Roles, Tablespaces and Instance Viewer are `P2`. Everything else is `P3`.

### 7.3 Real-Time SQL Monitor (Oracle only; Tuning Pack licence prompt)
List of monitored statements (status, duration, SQL ID, plan hash, user, parallel, DB time, I/O requests/bytes, buffer gets, start/end); detail = Overview, plan with per-line activity/predicates, Memory/Temp, Report XML. `P3`.

### 7.4 Tuning
| Feature | Pri |
|---|---|
| SQL Tuning Advisor from the worksheet (Alt+F11 run; Alt+F12 run scripts) | P3 |
| Hierarchical profiler (Profile… on PL/SQL: creates the profiler dir/tables if missing) | P3 |
| SQL Trace (.trc) viewer: list / statistics / tree views | P3 |
| Tuning Memory panel (SGA/PGA advisors, allocation, top PGA sessions) | P3 |
| Licensing prefs per connection: Tuning, Diagnostics, Change Management packs | P2 (needed to gate features) |

### 7.5 Snippets
Window with categories: Aggregate, Analytic, Character, Conversion, Date/Time, Numeric functions, Date Formats, Number Formats, Optimizer Hints, PL/SQL Programming Techniques, Pseudocolumns, Flashback, Predictive Analytics, SQL Developer Tips. Drag into editor; Add/Save Snippet (category, name, tooltip, text); Edit User Snippets. `P2`.

### 7.6 Code templates
Pref page: abbreviation → template text with template variables; expanded via completion. `P2`.

### 7.7 Database Cart
| Feature | W | M | Pri |
|---|---|---|---|
| New / Open / Save / Save As / Save All carts | Ctrl+N / Ctrl+O / Ctrl+S / Alt+A / Alt+L | Cmd+N/O/S, Alt+A/L | P3 |
| Add To Cart (from tree, most object types); columns Owner, Type, Name, Include DDL, Include Data, Where filter | | | P3 |
| Add initial/final script, add data file, delete row, select all | Ctrl+Shift+N/F/A/D, Ctrl+A | Cmd variants | P3 |
| Deploy (zip of scripts), Deploy Cloud, Export, Diff carts, Copy | Ctrl+D / Ctrl+E / Ctrl+I / Ctrl+C | Cmd variants | P3 |
| Cart tabs: previous/next, refresh | Shift+F5 / Ctrl+Shift+F5, Ctrl+R | | P3 |

### 7.8 Database Diff / Copy / Export / Import
| Feature | Pri |
|---|---|
| **Database Diff wizard**: source/destination connections; DDL options (schema, storage, terminator, pretty print, BYTE keyword, FORCE on views, constraints as ALTER, include DROP); object types (tables, views, indexes, constraints, sequences, procedures, functions, packages spec/body, triggers, types, synonyms, MVs, MV logs, db links, directory); specify objects; data diff; result: summary grid (type/owner/name/change), Visual DDL diff, generated ALTER/CREATE script | P2 |
| **Database Copy wizard**: Schemas copy / Objects copy / Tablespace copy; maintain or consolidate schemas; copy DDL (replace / don't replace existing), copy data (append, truncate, don't copy); ignore storage/partitioning; log + output panes | P2 |
| **Export wizard** (Tools ▸ Database Export): connection, Export DDL (options as Diff), Export Data with format (§6), types → objects → data filters (WHERE), output single file / separate files / by type / clipboard / worksheet, encoding, compressed | P2 |
| **Import Data wizard** (Table ▸ Import Data / Tables folder): file (CSV, delimited, text, XLS/XLSX), header, skip rows, preview limit, encoding, delimiter/enclosure; method: Insert, Insert Script, SQL*Loader, External Table, staging table; column mapping, type/date format, reject limit; create new table or append to an existing one | P2 |
| Quick DDL (Save to File/Worksheet/Clipboard), Generate DB Doc (HTML) | MVP (Quick DDL), P3 (DB Doc) |

### 7.9 Migration (to Oracle)
Sources: SQL Server, Sybase, DB2, Teradata, MySQL, PostgreSQL (via translation), MS Access, Redshift (cloud wizard). Features: migration repository (create/drop/truncate), Migration Wizard (capture → convert → translate → generate → move data online/offline), Quick Migrate, captured/converted model navigators (MD_* objects with Code/Columns/Indexes/Constraints tabs), set data-type mapping, offline scripts, translation scratch editor (translate SQL dialects, diff), SQL Translation Framework profiles/translators, application scan (finds DB SQL in app source files). All `P3`.

### 7.10 Data Modeler (embedded)
Logical / relational / physical / multidimensional / data-types / process models; diagrams (entity, table, view, FK, arcs, notes); engineer logical ↔ relational; import DDL/ERwin/Designer/data dictionary; DDL generation & comparison; design rules; domains, glossary, naming standards; reports; versioning via Subversion. Has its own keys (Ctrl+O/W/S, F5–F9 zoom, Alt+Shift+… tools, Ctrl+Shift+E export DDL, Ctrl+Shift+D import DDL). Table viewer **Model** tab = a read-only diagram of the table and its FK neighbours (`P2`). Everything else `P3`.

### 7.11 Unit Test
No unit-test extension is shipped in 26.2. Only error-text resources mention it. **Out of scope** (drop or `P3` as a new design).

### 7.12 Scheduler
Navigator nodes and actions in §3.1/§4. Also: New Job wizard (job type PL/SQL block / stored proc / named program / chain / external script; schedule immediate/once/repeating with a repeat-interval builder; destinations; notifications); Scheduler Design Editor (visual chain editor); Scheduler reports. `P2` for Jobs/Run Log/Enable/Disable/Run. Rest `P3`.

### 7.13 SSH
SSH navigator: SSH Hosts → Local/Remote Port Forwards; New SSH Host (name, host, port, username, key file, known-hosts file, OpenSSH config, strict key permissions, advanced overrides for ciphers/KEX/host key/MAC/auth methods/identities); Test, Test All, Connect, Disconnect, Remove. Connections can use a forward (connection type SSH). Prefs page SSH. `P2`.

### 7.14 REST / ORDS
REST Data Services navigator (connect to ORDS admin; New/Retrieve/Validate/Upload RESTful services); schema-level Enable/Drop REST Services; AutoREST "Enable REST Service" on table/view/package/proc/func; modules/templates/handlers editor with source + Test Handler; privileges & roles; REST definition export. `P3`.

### 7.15 PL/SQL Debugger
Compile for Debug, Debug (parameter dialog + generated block), breakpoints (F5 toggle; Breakpoints window), Data/Smart Data/Watches/Inspector/Stack windows, step controls, Remote Debug (listen for JDWP from the connection menu). Prefs: Debugger (port range, prompt for host), Breakpoints, Default Actions, Data/Inspector/Smart Data/Stack/ToolTip/Watches. `P3`.

### 7.16 Find DB Object
Dockable search: connection, name pattern (auto-append `%` pref), object types (tables, views, PL/SQL source text, columns, synonyms…), owner, search in all schemas; results tree, click opens the object/source line. `P2`.

### 7.17 File Navigator, Recent Objects, Task Progress
File system browser (open .sql, refresh, delete; Tag SQL, Obfuscate, SQL Patroller); Recent Objects window; Task Progress (background tasks: cancel, pause). `P2`.

### 7.18 Spatial, RDF Graph, PGQL, OLAP, Analytic Views, TimesTen, Data Miner, Change Management (EM), Swift/Object Storage, Hadoop connectors
Spatial map viewer (map tabs, geometry query layers, zoom; Ctrl+T/N/D/R/I/O/F/M) plus spatial metadata/index/validation actions. RDF networks/models/rulebases with SPARQL editor (Ctrl+F5 run). PGQL worksheet. OLAP AW/cube build. Analytic View designer (HCS). TimesTen cache groups, replication, Scaleout grid navigator, Index Advisor (Ctrl+Shift+X). Data Miner workflows. EM Change Management plans. Swift object storage. Hadoop (Oracle SQL Connector for HDFS, Copy to Hadoop). **All `P3`. Most are Oracle-option specific.**

### 7.19 Security extras
Data Redaction (add/alter, enable/disable, drop policy on table/view/MV/column), column Encrypt/Decrypt (TDE), Security reports. `P3`.

### 7.20 Preferences tree (top-level sections + key options)
| Section | Key options | Pri |
|---|---|---|
| Environment | look & feel, encoding, line terminator, undo levels, log/confirmation dialogs, Automatically reload externally modified files | MVP (encoding, theme) |
| Accelerators / Shortcut Keys | keymap presets (Default, Default MacOS X, Classic, Emacs, Visual C++, Eclipse), per-action rebinding, import/export | P2 |
| Code Editor | Bookmarks, Caret Behavior, Display (line numbers, whitespace, right margin), Line Gutter, Fonts, PL/SQL Syntax Colors, Custom Syntax Rules, Completion Insight (auto-popup delay, case), Format / Advanced Format / Custom Format (SQL formatter rules), Parser, Undo Behavior, Save actions | MVP (fonts, colors, line numbers, completion), P2 (formatter) |
| Database | Autocommit; SQL Array Fetch Size; Display Null As / null background color; struct/XML in grid; open object on single click; append `%` in search; Use OCI/Thick driver; Kerberos thin config (krb5.conf, credential cache); **Tnsnames Directory**; SQLPATH; persist file names; Licensing; Navigation Filter (object types shown in tree, per connection); Third Party JDBC Drivers; User Defined Extensions; External editor for BLOBs | MVP (autocommit, fetch size, null display, tns dir, drivers) |
| ▸ Worksheet | open worksheet on connect; new worksheet uses unshared connection; close worksheets on disconnect; prompt save on close; zebra grid; max rows printed in script; max lines in Script Output; SQL History limit; default script path; save bind variables on exit; show query results in new tabs; re-initialise on EXIT; skip Query Builder metadata | MVP |
| ▸ Autotrace/Explain Plan | columns to show (object name, options, cost, cardinality, bytes, last rows/gets/elapsed/starts, predicates, projection, other XML); V$MYSTAT stats; skip zero/NULL; fetch all rows; plan search suggestions | MVP (explain columns), P2 |
| ▸ Object Viewer | auto-freeze viewer windows; DDL options: show storage, terminator, pretty format, BYTE keyword, FORCE, constraints as ALTER, schema | MVP (DDL options), P2 |
| ▸ NLS | language, territory, characterset (read-only), sort, comp, calendar, date language, date/timestamp/TZ formats, numeric chars, currency, length semantics | MVP (date/timestamp formats), P2 |
| ▸ PL/SQL Compiler | warning categories (severe/performance/informational), optimise level, PL/Scope identifiers/statements | P3 |
| ▸ Drag And Drop | object name only / SELECT / INSERT / UPDATE / DELETE / JOIN / individual statements / prompt every time | P2 |
| ▸ Advanced | data editor: post edits on row change, autocommit; chart row limit; autoformat dictionary SQL; confirmations to log | P2 |
| ▸ Code Templates | template list | P2 |
| ▸ SQL Formatter | Oracle Formatting (rules, import/export profile) + Other Vendors (Sybase, Access, SQL Server, DB2) | P2 |
| ▸ Utilities ▸ Cart / Difference / Export / Import | default dirs; export format defaults per format (Delimited, CSV, Insert, SQL*Loader, Text, Excel, Fixed, HTML, XML); import file formats; import methods (Insert, SQL Loader, External Table) | P2 |
| ▸ SQL Code Advisor, Licensing, External Editor, File Chooser | | P3 |
| Debugger | port range, breakpoints, default actions, data/inspector/watches/stack/tooltip | P3 |
| Reports | close reports on disconnect, chart row limit, report location | P2 |
| SSH | known hosts, OpenSSH config, algorithms | P2 |
| Cart, Instance Viewer, Migration (generation, data move, identifiers, translators), Data Modeler (many), Data Miner, OLAP, Analytic View, Change Management, Cloud Migrations, Swift, Usage Reporting | | P3 |
| Versioning (Git/Subversion), Web Browser and Proxy, Global Ignore list, Mouseover popups | | P3 |

---

## 8. Cross-cutting behaviours worth matching

| Behaviour | Pri |
|---|---|
| Shared vs unshared sessions: worksheets share the connection's session by default. An unshared worksheet opens its own session, so commit/rollback is separate | MVP |
| Transaction indicator: uncommitted-changes prompt on disconnect or close | MVP |
| Background tasks with cancel (query, script, export), shown in Task Progress | MVP (cancel) |
| Dockable/tabbed window layout, editor tab groups, floating windows | MVP (dock+tabs), P2 (float) |
| Persisted state: open worksheets, connections, history, grid settings, prefs (user config dir) | MVP |
| Every object action dialog has two tabs: Prompts (parameters) and SQL (preview of the statement that will run) before Apply | MVP — matches the "show SQL before execute" pattern |
| Feature gating by DB vendor/version: many actions appear only for connType Oracle, or "Oracle DBA" (DBA panel), or need a minimum version (e.g. duality views, PDBs, 12.2+ details) | MVP (design the registry for it) |
| Licence gating: Tuning/Diagnostics packs per connection (SQL Monitor, ASH/AWR, ADDM, Tuning Advisor) | P2 |
