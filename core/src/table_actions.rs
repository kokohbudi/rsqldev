//! Navigator table actions (SQL Developer's Table ▸ / Column ▸ / Constraint ▸ / Index ▸ /
//! Trigger ▸ submenus): each builds the exact statements the confirmation shows. Names go
//! through `quote` / `ident`, text through `lit`, numbers are checked. `Err` = the engine has
//! no such statement (the menu item says so).

use crate::ddl::{self, ColumnDef, ConKind, ConstraintDef, col_list, engine, ident, q};
use crate::dialect::{fill, lit, quote};
use crate::registry::Kind as Db;

type R<T> = Result<T, String>;

/// Table as `"OWNER"."NAME"` (stored names, as listed in the navigator).
pub fn table(db: Db, owner: &str, name: &str) -> String {
    if owner.is_empty() { quote(db, name) } else { format!("{}.{}", quote(db, owner), quote(db, name)) }
}

fn no(db: Db, what: &str) -> String {
    format!("{} has no {what}", engine(db))
}

// ---- Table ----

pub fn rename(db: Db, owner: &str, name: &str, new: &str) -> R<String> {
    let t = table(db, owner, name);
    let n = ident(db, new);
    Ok(match db {
        Db::Mssql => format!("EXEC sp_rename N'{}', N'{}'", lit(&format!("{owner}.{name}")), lit(&n)),
        Db::Mysql => format!("RENAME TABLE {t} TO {}", table(db, owner, &n)),
        _ => format!("ALTER TABLE {t} RENAME TO {}", quote(db, &n)),
    })
}

/// Rename… on a view / sequence / synonym / trigger node. `schema` = the session's current
/// schema (Oracle RENAME works on its own objects only).
pub fn rename_object(db: Db, kind: &str, owner: &str, name: &str, new: &str, schema: &str) -> R<String> {
    use crate::dialect::{SEQUENCE, SYNONYM, TRIGGER, VIEW};
    let (t, n) = (table(db, owner, name), ident(db, new));
    if n.is_empty() {
        return Err("A name is needed".into());
    }
    match (db, kind) {
        (Db::Oracle, TRIGGER) => Ok(format!("ALTER TRIGGER {t} RENAME TO {}", quote(db, &n))),
        (Db::Oracle, VIEW | SEQUENCE | SYNONYM) if owner.eq_ignore_ascii_case(schema) => Ok(format!("RENAME {} TO {}", quote(db, name), quote(db, &n))),
        (Db::Oracle, VIEW | SEQUENCE | SYNONYM) => Err(format!("Oracle renames only its own objects: connect as {owner}")),
        (Db::Postgres, VIEW | SEQUENCE) => Ok(format!("ALTER {kind} {t} RENAME TO {}", quote(db, &n))),
        (Db::Mysql, VIEW) => Ok(format!("RENAME TABLE {t} TO {}", table(db, owner, &n))),
        _ => Err(no(db, &format!("rename for a {}", kind.to_lowercase()))),
    }
}

/// Enable / Disable on an Oracle trigger node.
pub fn trigger_state(db: Db, owner: &str, name: &str, on: bool) -> R<String> {
    match db {
        Db::Oracle => Ok(format!("ALTER TRIGGER {} {}", table(db, owner, name), if on { "ENABLE" } else { "DISABLE" })),
        _ => Err(no(db, "trigger enable / disable without its table (use the table's Trigger ▸ menu)")),
    }
}

/// Copy…: a new table with the same columns, with or without the rows.
pub fn copy(db: Db, owner: &str, name: &str, new_owner: &str, new_name: &str, data: bool) -> R<Vec<String>> {
    let src = table(db, owner, name);
    let dst = table(db, &ident(db, new_owner), &ident(db, new_name));
    let filter = if data { "" } else { " WHERE 1 = 0" };
    Ok(match db {
        Db::Mysql => {
            let mut v = vec![format!("CREATE TABLE {dst} LIKE {src}")];
            if data {
                v.push(format!("INSERT INTO {dst} SELECT * FROM {src}"));
            }
            v
        }
        Db::Mssql => vec![format!("SELECT * INTO {dst} FROM {src}{filter}")],
        Db::Clickhouse => {
            let mut v = vec![format!("CREATE TABLE {dst} AS {src}")];
            if data {
                v.push(format!("INSERT INTO {dst} SELECT * FROM {src}"));
            }
            v
        }
        _ => vec![format!("CREATE TABLE {dst} AS SELECT * FROM {src}{filter}")],
    })
}

pub fn drop(db: Db, owner: &str, name: &str, cascade: bool, purge: bool) -> R<String> {
    let t = table(db, owner, name);
    Ok(match db {
        Db::Oracle => format!("DROP TABLE {t}{}{}", if cascade { " CASCADE CONSTRAINTS" } else { "" }, if purge { " PURGE" } else { "" }),
        Db::Postgres | Db::Duckdb if cascade => format!("DROP TABLE {t} CASCADE"),
        _ => format!("DROP TABLE {t}"),
    })
}

/// Truncate…: Oracle keeps (REUSE) or frees (DROP) the storage.
pub fn truncate(db: Db, owner: &str, name: &str, reuse: bool) -> R<String> {
    let t = table(db, owner, name);
    Ok(match db {
        Db::Oracle => format!("TRUNCATE TABLE {t} {} STORAGE", if reuse { "REUSE" } else { "DROP" }),
        Db::Sqlite => format!("DELETE FROM {t}"),
        _ => format!("TRUNCATE TABLE {t}"),
    })
}

pub fn lock_modes(db: Db) -> &'static [&'static str] {
    match db {
        Db::Oracle => &["ROW SHARE", "ROW EXCLUSIVE", "SHARE UPDATE", "SHARE", "SHARE ROW EXCLUSIVE", "EXCLUSIVE"],
        Db::Postgres => &["ACCESS SHARE", "ROW SHARE", "ROW EXCLUSIVE", "SHARE UPDATE EXCLUSIVE", "SHARE", "SHARE ROW EXCLUSIVE", "EXCLUSIVE", "ACCESS EXCLUSIVE"],
        Db::Mysql => &["READ", "WRITE"],
        _ => &[],
    }
}

/// Lock…: held until COMMIT / ROLLBACK (MySQL: UNLOCK TABLES) on the worksheet session.
pub fn lock(db: Db, owner: &str, name: &str, mode: &str, nowait: bool) -> R<String> {
    if !lock_modes(db).contains(&mode) {
        return Err(format!("lock mode {mode}: {}", no(db, "such lock mode")));
    }
    let t = table(db, owner, name);
    Ok(match db {
        Db::Mysql => format!("LOCK TABLES {t} {mode}"),
        _ => format!("LOCK TABLE {t} IN {mode} MODE{}", if nowait { " NOWAIT" } else { "" }),
    })
}

pub fn comment(db: Db, owner: &str, name: &str, text: &str) -> R<String> {
    ddl::table_comment_sql(db, &table(db, owner, name), text.trim()).ok_or_else(|| no(db, "table comments here"))
}

/// Parallel… (degree) / No Parallel… (None). Oracle only.
pub fn parallel(db: Db, owner: &str, name: &str, degree: Option<&str>) -> R<String> {
    if db != Db::Oracle {
        return Err(no(db, "PARALLEL"));
    }
    let t = table(db, owner, name);
    Ok(match degree.map(str::trim) {
        Some(d) if !d.is_empty() => {
            if !d.chars().all(|c| c.is_ascii_digit()) {
                return Err(format!("degree must be a number, not '{d}'"));
            }
            format!("ALTER TABLE {t} PARALLEL {d}")
        }
        Some(_) => format!("ALTER TABLE {t} PARALLEL"),
        None => format!("ALTER TABLE {t} NOPARALLEL"),
    })
}

pub fn logging(db: Db, owner: &str, name: &str, on: bool) -> R<String> {
    if db != Db::Oracle {
        return Err(no(db, "LOGGING"));
    }
    Ok(format!("ALTER TABLE {} {}", table(db, owner, name), if on { "LOGGING" } else { "NOLOGGING" }))
}

// ---- Column ----

pub fn add_column(db: Db, owner: &str, name: &str, c: &ColumnDef) -> R<String> {
    let d = ddl::col_def(db, c)?;
    let t = table(db, owner, name);
    Ok(match db {
        Db::Oracle => format!("ALTER TABLE {t} ADD ({d})"),
        Db::Mssql => format!("ALTER TABLE {t} ADD {d}"),
        _ => format!("ALTER TABLE {t} ADD COLUMN {d}"),
    })
}

pub fn rename_column(db: Db, owner: &str, name: &str, col: &str, new: &str) -> R<String> {
    let n = ident(db, new);
    Ok(match db {
        Db::Mssql => format!("EXEC sp_rename N'{}', N'{}', 'COLUMN'", lit(&format!("{}.{}", table(db, owner, name), quote(db, col))), lit(&n)),
        _ => format!("ALTER TABLE {} RENAME COLUMN {} TO {}", table(db, owner, name), quote(db, col), quote(db, &n)),
    })
}

pub fn drop_column(db: Db, owner: &str, name: &str, col: &str) -> R<String> {
    Ok(format!("ALTER TABLE {} DROP COLUMN {}", table(db, owner, name), quote(db, col)))
}

pub fn comment_column(db: Db, owner: &str, name: &str, col: &str, text: &str) -> R<String> {
    ddl::comment_sql(db, &table(db, owner, name), col, text.trim()).ok_or_else(|| format!("{}: change column comments in Edit Table", no(db, "COMMENT ON COLUMN")))
}

// ---- Constraint ----

/// Add Primary Key… / Unique… / Foreign Key… / Check…: `kind` None = primary key.
pub fn add_constraint(db: Db, owner: &str, name: &str, kind: Option<ConKind>, cname: &str, columns: &str, ref_table: &str, ref_columns: &str, check: &str) -> R<String> {
    if !ddl::alters_constraints(db) {
        return Err(format!("{} cannot add constraints to an existing table", engine(db)));
    }
    let t = table(db, owner, name);
    let named = if cname.trim().is_empty() { String::new() } else { format!("CONSTRAINT {} ", q(db, cname)) };
    let body = match kind {
        None => format!("{named}PRIMARY KEY ({})", col_list(db, columns, false)?),
        Some(kind) => ddl::con_def(db, &ConstraintDef { name: cname.into(), kind, columns: columns.into(), ref_table: ref_table.into(), ref_columns: ref_columns.into(), check: check.into(), ..Default::default() })?,
    };
    Ok(format!("ALTER TABLE {t} ADD {body}"))
}

/// Enable / Disable one constraint of `table_q` (any table: related foreign keys too).
/// Oracle `validate`: VALIDATE (check the rows) or NOVALIDATE.
pub fn enable_constraint(db: Db, table_q: &str, cname: &str, enable: bool, validate: bool) -> R<String> {
    let n = quote(db, cname);
    match db {
        Db::Oracle => Ok(format!("ALTER TABLE {table_q} {} {} CONSTRAINT {n}", if enable { "ENABLE" } else { "DISABLE" }, if validate { "VALIDATE" } else { "NOVALIDATE" })),
        Db::Mssql => Ok(format!("ALTER TABLE {table_q} {} CONSTRAINT {n}", if enable { if validate { "WITH CHECK CHECK" } else { "CHECK" } } else { "NOCHECK" })),
        _ => Err(no(db, "constraint enable / disable")),
    }
}

pub fn rename_constraint(db: Db, owner: &str, name: &str, cname: &str, new: &str) -> R<String> {
    ddl::rename_constraint(db, &table(db, owner, name), owner, cname, new)
}

pub fn drop_constraint(db: Db, owner: &str, name: &str, cname: &str, kind: &str) -> R<String> {
    if !ddl::alters_constraints(db) {
        return Err(format!("{} cannot drop constraints of an existing table", engine(db)));
    }
    let n = quote(db, cname);
    let t = table(db, owner, name);
    Ok(match (db, kind) {
        (Db::Mysql, "PRIMARY KEY") => format!("ALTER TABLE {t} DROP PRIMARY KEY"),
        (Db::Mysql, "FOREIGN KEY") => format!("ALTER TABLE {t} DROP FOREIGN KEY {n}"),
        (Db::Mysql, "UNIQUE") => format!("ALTER TABLE {t} DROP INDEX {n}"),
        (Db::Mysql, _) => format!("ALTER TABLE {t} DROP CHECK {n}"),
        _ => format!("ALTER TABLE {t} DROP CONSTRAINT {n}"),
    })
}

/// Foreign keys of other tables pointing at this table: rows owner, table, constraint.
pub fn related_fks_sql(db: Db, owner: &str, name: &str) -> Option<String> {
    let tpl = match db {
        Db::Oracle => "SELECT c.owner, c.table_name, c.constraint_name FROM all_constraints c WHERE c.constraint_type = 'R' AND (c.r_owner, c.r_constraint_name) IN (SELECT owner, constraint_name FROM all_constraints WHERE owner = '{o}' AND table_name = '{n}' AND constraint_type IN ('P', 'U')) ORDER BY 1, 2, 3",
        Db::Mssql => "SELECT OBJECT_SCHEMA_NAME(f.parent_object_id), OBJECT_NAME(f.parent_object_id), f.name FROM sys.foreign_keys f WHERE f.referenced_object_id = OBJECT_ID('{lq}') ORDER BY 1, 2, 3",
        _ => return None,
    };
    Some(fill(db, tpl, owner, name))
}

// ---- Index ----

pub fn rebuild_index(db: Db, owner: &str, name: &str, index: &str, online: bool) -> R<String> {
    let s = ddl::rebuild_index(db, owner, name, index).ok_or_else(|| no(db, "index rebuild"))?;
    Ok(if online && db == Db::Oracle { s + " ONLINE" } else { s })
}

// ---- Trigger ----

/// Triggers of the table: one name per row.
pub fn triggers_sql(db: Db, owner: &str, name: &str) -> Option<String> {
    let tpl = match db {
        Db::Oracle => "SELECT trigger_name, status FROM all_triggers WHERE table_owner = '{o}' AND table_name = '{n}' ORDER BY 1",
        Db::Postgres => "SELECT tgname, CASE tgenabled WHEN 'D' THEN 'DISABLED' ELSE 'ENABLED' END FROM pg_trigger WHERE tgrelid = '{lq}'::regclass AND NOT tgisinternal ORDER BY 1",
        Db::Mysql => "SELECT trigger_name, 'ENABLED' FROM information_schema.triggers WHERE event_object_schema = '{o}' AND event_object_table = '{n}' ORDER BY 1",
        Db::Mssql => "SELECT name, CASE is_disabled WHEN 1 THEN 'DISABLED' ELSE 'ENABLED' END FROM sys.triggers WHERE parent_id = OBJECT_ID('{lq}') ORDER BY 1",
        Db::Sqlite => "SELECT name, 'ENABLED' FROM sqlite_master WHERE type = 'trigger' AND tbl_name = '{n}' ORDER BY 1",
        _ => return None,
    };
    Some(fill(db, tpl, owner, name))
}

/// Create…: a trigger skeleton for the worksheet.
pub fn trigger_template(db: Db, owner: &str, name: &str) -> R<String> {
    let t = table(db, owner, name);
    let tn = format!("{}_TRG", name.trim_matches('"'));
    Ok(match db {
        Db::Oracle => format!("CREATE OR REPLACE TRIGGER {}\nBEFORE INSERT OR UPDATE ON {t}\nFOR EACH ROW\nBEGIN\n  NULL;\nEND;\n/\n", table(db, owner, &tn)),
        Db::Postgres => format!("CREATE OR REPLACE FUNCTION {}() RETURNS trigger LANGUAGE plpgsql AS $$\nBEGIN\n  RETURN NEW;\nEND $$;\n\nCREATE TRIGGER {} BEFORE INSERT OR UPDATE ON {t}\nFOR EACH ROW EXECUTE FUNCTION {}();\n", table(db, owner, &format!("{tn}_fn").to_lowercase()), quote(db, &tn.to_lowercase()), table(db, owner, &format!("{tn}_fn").to_lowercase())),
        Db::Mysql => format!("CREATE TRIGGER {} BEFORE INSERT ON {t}\nFOR EACH ROW\nSET NEW.col = NEW.col;\n", table(db, owner, &tn)),
        Db::Mssql => format!("CREATE TRIGGER {} ON {t}\nAFTER INSERT, UPDATE\nAS\nBEGIN\n  SET NOCOUNT ON;\nEND\nGO\n", table(db, owner, &tn)),
        Db::Sqlite => format!("CREATE TRIGGER {} AFTER INSERT ON {}\nBEGIN\n  SELECT 1;\nEND;\n", quote(db, &tn), quote(db, name)),
        _ => return Err(no(db, "triggers")),
    })
}

/// Create (PK from Sequence)…: fill the key column from a sequence when it is NULL. Oracle.
pub fn pk_from_sequence(db: Db, owner: &str, name: &str, trigger: &str, sequence: &str, column: &str) -> R<String> {
    if db != Db::Oracle {
        return Err(no(db, "sequence triggers here"));
    }
    let (tr, seq, col) = (ident(db, trigger), ident(db, sequence), quote(db, &ident(db, column)));
    if tr.is_empty() || seq.is_empty() || column.trim().is_empty() {
        return Err("trigger, sequence and column are all needed".into());
    }
    Ok(format!(
        "CREATE OR REPLACE TRIGGER {}\nBEFORE INSERT ON {}\nFOR EACH ROW\nBEGIN\n  IF :NEW.{col} IS NULL THEN\n    :NEW.{col} := {}.NEXTVAL;\n  END IF;\nEND;",
        table(db, owner, &tr),
        table(db, owner, name),
        table(db, owner, &seq)
    ))
}

/// Enable All / Disable All triggers of the table.
pub fn enable_triggers(db: Db, owner: &str, name: &str, enable: bool) -> R<String> {
    let t = table(db, owner, name);
    let on = if enable { "ENABLE" } else { "DISABLE" };
    Ok(match db {
        Db::Oracle => format!("ALTER TABLE {t} {on} ALL TRIGGERS"),
        // USER: not the internal foreign key triggers (those need a superuser).
        Db::Postgres => format!("ALTER TABLE {t} {on} TRIGGER USER"),
        Db::Mssql => format!("{on} TRIGGER ALL ON {t}"),
        _ => return Err(no(db, "trigger enable / disable")),
    })
}

pub fn enable_trigger(db: Db, owner: &str, name: &str, trigger: &str, enable: bool) -> R<String> {
    let on = if enable { "ENABLE" } else { "DISABLE" };
    Ok(match db {
        Db::Oracle => format!("ALTER TRIGGER {} {on}", table(db, owner, trigger)),
        Db::Postgres => format!("ALTER TABLE {} {on} TRIGGER {}", table(db, owner, name), quote(db, trigger)),
        Db::Mssql => format!("{on} TRIGGER {} ON {}", table(db, owner, trigger), table(db, owner, name)),
        _ => return Err(no(db, "trigger enable / disable")),
    })
}

pub fn drop_trigger(db: Db, owner: &str, name: &str, trigger: &str) -> R<String> {
    Ok(match db {
        Db::Postgres => format!("DROP TRIGGER {} ON {}", quote(db, trigger), table(db, owner, name)),
        Db::Sqlite => format!("DROP TRIGGER {}", quote(db, trigger)),
        Db::Duckdb | Db::Clickhouse => return Err(no(db, "triggers")),
        _ => format!("DROP TRIGGER {}", table(db, owner, trigger)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_renames() {
        assert_eq!(rename_object(Db::Oracle, "TRIGGER", "HR", "T1", "t2", "SCOTT").unwrap(), r#"ALTER TRIGGER "HR"."T1" RENAME TO "T2""#);
        assert_eq!(rename_object(Db::Oracle, "VIEW", "HR", "V", "w", "hr").unwrap(), r#"RENAME "V" TO "W""#);
        assert!(rename_object(Db::Oracle, "SEQUENCE", "HR", "S", "s2", "SCOTT").is_err());
        assert_eq!(rename_object(Db::Postgres, "SEQUENCE", "public", "s", "S2", "public").unwrap(), r#"ALTER SEQUENCE "public"."s" RENAME TO "s2""#);
        assert!(rename_object(Db::Mssql, "VIEW", "dbo", "v", "w", "dbo").is_err());
        assert!(rename_object(Db::Oracle, "TRIGGER", "HR", "T1", " ", "HR").is_err());
        assert_eq!(trigger_state(Db::Oracle, "HR", "T1", false).unwrap(), r#"ALTER TRIGGER "HR"."T1" DISABLE"#);
    }

    #[test]
    fn table_actions() {
        assert_eq!(drop(Db::Oracle, "HR", "EMP", true, true).unwrap(), r#"DROP TABLE "HR"."EMP" CASCADE CONSTRAINTS PURGE"#);
        assert_eq!(truncate(Db::Oracle, "HR", "EMP", true).unwrap(), r#"TRUNCATE TABLE "HR"."EMP" REUSE STORAGE"#);
        assert_eq!(copy(Db::Oracle, "HR", "EMP", "hr", "emp2", false).unwrap(), vec![r#"CREATE TABLE "HR"."EMP2" AS SELECT * FROM "HR"."EMP" WHERE 1 = 0"#]);
        assert_eq!(copy(Db::Mysql, "db", "t", "db", "t2", true).unwrap().len(), 2);
        assert_eq!(lock(Db::Oracle, "HR", "EMP", "SHARE", true).unwrap(), r#"LOCK TABLE "HR"."EMP" IN SHARE MODE NOWAIT"#);
        assert!(lock(Db::Oracle, "HR", "EMP", "SHARE; DROP", false).is_err());
        assert_eq!(parallel(Db::Oracle, "HR", "EMP", Some("4")).unwrap(), r#"ALTER TABLE "HR"."EMP" PARALLEL 4"#);
        assert!(parallel(Db::Oracle, "HR", "EMP", Some("4 x")).is_err() && parallel(Db::Mysql, "a", "b", None).is_err());
        assert_eq!(comment(Db::Oracle, "HR", "EMP", "o'k").unwrap(), r#"COMMENT ON TABLE "HR"."EMP" IS 'o''k'"#);
        assert_eq!(rename(Db::Postgres, "s", "t", "New").unwrap(), r#"ALTER TABLE "s"."t" RENAME TO "new""#);
    }

    #[test]
    fn column_constraint_index_trigger() {
        let c = ColumnDef { name: "age".into(), ty: "NUMBER".into(), size: "3".into(), ..Default::default() };
        assert_eq!(add_column(Db::Oracle, "HR", "EMP", &c).unwrap(), r#"ALTER TABLE "HR"."EMP" ADD ("AGE" NUMBER(3))"#);
        assert_eq!(rename_column(Db::Mssql, "dbo", "t", "a", "b").unwrap(), "EXEC sp_rename N'[dbo].[t].[a]', N'b', 'COLUMN'");
        assert!(comment_column(Db::Mysql, "d", "t", "a", "x").is_err());
        assert_eq!(add_constraint(Db::Oracle, "HR", "EMP", None, "emp_pk", "id, b", "", "", "").unwrap(), r#"ALTER TABLE "HR"."EMP" ADD CONSTRAINT "EMP_PK" PRIMARY KEY ("ID", "B")"#);
        assert_eq!(add_constraint(Db::Oracle, "HR", "EMP", Some(ConKind::Check), "c1", "", "", "", "sal > 0").unwrap(), r#"ALTER TABLE "HR"."EMP" ADD CONSTRAINT "C1" CHECK (sal > 0)"#);
        assert!(add_constraint(Db::Sqlite, "main", "t", None, "", "a", "", "", "").is_err());
        assert_eq!(enable_constraint(Db::Oracle, r#""HR"."EMP""#, "C1", false, false).unwrap(), r#"ALTER TABLE "HR"."EMP" DISABLE NOVALIDATE CONSTRAINT "C1""#);
        assert_eq!(drop_constraint(Db::Mysql, "d", "t", "fk1", "FOREIGN KEY").unwrap(), "ALTER TABLE `d`.`t` DROP FOREIGN KEY `fk1`");
        assert_eq!(rebuild_index(Db::Oracle, "HR", "EMP", "IX", true).unwrap(), r#"ALTER INDEX "HR"."IX" REBUILD ONLINE"#);
        let t = pk_from_sequence(Db::Oracle, "HR", "EMP", "emp_bi", "emp_seq", "id").unwrap();
        assert!(t.contains(r#"BEFORE INSERT ON "HR"."EMP""#) && t.contains(r#":NEW."ID" := "HR"."EMP_SEQ".NEXTVAL;"#), "{t}");
        assert_eq!(enable_triggers(Db::Postgres, "s", "t", false).unwrap(), r#"ALTER TABLE "s"."t" DISABLE TRIGGER USER"#);
        assert_eq!(drop_trigger(Db::Postgres, "s", "t", "tr").unwrap(), r#"DROP TRIGGER "tr" ON "s"."t""#);
        assert!(triggers_sql(Db::Duckdb, "m", "t").is_none());
    }
}
