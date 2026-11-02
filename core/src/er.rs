//! Tools ▸ ER Diagram: a schema's tables (columns, primary keys) and foreign keys as an SVG
//! file. Two queries per schema, then a grid layout: no layout engine, no dependency.

use crate::dialect::fill;
use crate::registry::Kind as Db;
use std::collections::BTreeMap;

/// (columns query: table, column, type, "Y" when in the primary key; foreign keys query:
/// table, referenced table). Templates use `{o}` (the schema).
pub fn queries(db: Db, owner: &str) -> Option<(String, String)> {
    let (cols, fks) = match db {
        Db::Oracle => (
            "SELECT c.table_name, c.column_name, c.data_type, CASE WHEN EXISTS (SELECT 1 FROM all_constraints k JOIN all_cons_columns kc ON kc.owner = k.owner AND kc.constraint_name = k.constraint_name WHERE k.owner = c.owner AND k.table_name = c.table_name AND k.constraint_type = 'P' AND kc.column_name = c.column_name) THEN 'Y' END FROM all_tab_columns c JOIN all_tables t ON t.owner = c.owner AND t.table_name = c.table_name WHERE c.owner = '{o}' ORDER BY c.table_name, c.column_id",
            "SELECT c.table_name, r.table_name FROM all_constraints c JOIN all_constraints r ON r.owner = c.r_owner AND r.constraint_name = c.r_constraint_name WHERE c.owner = '{o}' AND c.constraint_type = 'R'",
        ),
        Db::Postgres => (
            "SELECT c.table_name, c.column_name, c.data_type, CASE WHEN EXISTS (SELECT 1 FROM information_schema.table_constraints k JOIN information_schema.key_column_usage u ON u.constraint_name = k.constraint_name AND u.table_schema = k.table_schema WHERE k.table_schema = c.table_schema AND k.table_name = c.table_name AND k.constraint_type = 'PRIMARY KEY' AND u.column_name = c.column_name) THEN 'Y' END FROM information_schema.columns c JOIN information_schema.tables t ON t.table_schema = c.table_schema AND t.table_name = c.table_name AND t.table_type = 'BASE TABLE' WHERE c.table_schema = '{o}' ORDER BY c.table_name, c.ordinal_position",
            "SELECT cl.relname, rl.relname FROM pg_constraint k JOIN pg_class cl ON cl.oid = k.conrelid JOIN pg_class rl ON rl.oid = k.confrelid JOIN pg_namespace n ON n.oid = cl.relnamespace WHERE k.contype = 'f' AND n.nspname = '{o}'",
        ),
        Db::Mysql => (
            "SELECT c.table_name, c.column_name, c.data_type, CASE WHEN c.column_key = 'PRI' THEN 'Y' END FROM information_schema.columns c JOIN information_schema.tables t ON t.table_schema = c.table_schema AND t.table_name = c.table_name AND t.table_type = 'BASE TABLE' WHERE c.table_schema = '{o}' ORDER BY c.table_name, c.ordinal_position",
            "SELECT DISTINCT table_name, referenced_table_name FROM information_schema.key_column_usage WHERE table_schema = '{o}' AND referenced_table_name IS NOT NULL",
        ),
        Db::Mssql => (
            "SELECT t.name, c.name, TYPE_NAME(c.user_type_id), CASE WHEN EXISTS (SELECT 1 FROM sys.indexes i JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id WHERE i.is_primary_key = 1 AND ic.object_id = c.object_id AND ic.column_id = c.column_id) THEN 'Y' END FROM sys.columns c JOIN sys.tables t ON t.object_id = c.object_id WHERE t.schema_id = SCHEMA_ID('{o}') ORDER BY t.name, c.column_id",
            "SELECT OBJECT_NAME(f.parent_object_id), OBJECT_NAME(f.referenced_object_id) FROM sys.foreign_keys f WHERE OBJECT_SCHEMA_NAME(f.parent_object_id) = '{o}'",
        ),
        Db::Sqlite => (
            "SELECT m.name, p.name, p.type, CASE WHEN p.pk > 0 THEN 'Y' END FROM \"{o}\".sqlite_master m, pragma_table_info(m.name, '{o}') p WHERE m.type = 'table' AND m.name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY m.name, p.cid",
            "SELECT m.name, f.\"table\" FROM \"{o}\".sqlite_master m, pragma_foreign_key_list(m.name, '{o}') f WHERE m.type = 'table'",
        ),
        Db::Duckdb => (
            "SELECT c.table_name, c.column_name, c.data_type, CASE WHEN EXISTS (SELECT 1 FROM duckdb_constraints() k WHERE k.schema_name = c.schema_name AND k.table_name = c.table_name AND k.constraint_type = 'PRIMARY KEY' AND list_contains(k.constraint_column_names, c.column_name)) THEN 'Y' END FROM duckdb_columns() c JOIN duckdb_tables() t ON t.schema_name = c.schema_name AND t.table_name = c.table_name WHERE c.schema_name = '{o}' ORDER BY c.table_name, c.column_index",
            "SELECT table_name, referenced_table FROM duckdb_constraints() WHERE schema_name = '{o}' AND constraint_type = 'FOREIGN KEY'",
        ),
        Db::Clickhouse => return None,
    };
    // SQLite quotes the schema as an identifier in the FROM clause.
    let o = |s: &str| if db == Db::Sqlite { s.replace("\"{o}\"", &crate::dialect::quote(db, owner)) } else { s.to_string() };
    Some((fill(db, &o(cols), owner, ""), fill(db, &o(fks), owner, "")))
}

/// Tools ▸ ER Diagram end to end on a connection of its own, in the calling thread (the app
/// runs it on a thread of its own): the app's sessions stay free however long the dictionary
/// queries take on a big schema, and the connection closes when this returns. `schema` empty
/// = the connection's current schema; `pattern` filters table names ([`crate::config::name_matches`]).
/// `cancel` holds the running statement's canceller, `stop` is checked between steps, `step`
/// reports progress. Returns (schema, SVG, tables drawn), or None when there is nothing to draw.
#[allow(clippy::too_many_arguments)]
pub fn generate(
    cfg: &crate::config::ConnConfig,
    password: &str,
    prefs: &crate::config::Prefs,
    schema: &str,
    pattern: &str,
    cancel: &std::sync::Mutex<Option<crate::Canceller>>,
    stop: &std::sync::atomic::AtomicBool,
    step: &dyn Fn(&str),
) -> anyhow::Result<Option<(String, String, usize)>> {
    use std::sync::atomic::Ordering;
    let _busy = crate::worker::Busy::new();
    let db = crate::registry::find(&cfg.driver).map(|d| d.backend).ok_or_else(|| anyhow::anyhow!("Unknown driver {}", cfg.driver))?;
    step("Connecting…");
    let mut b = crate::backend::connect(cfg, password, prefs)?;
    *cancel.lock().unwrap() = b.canceller();
    let stopped = || stop.load(Ordering::Relaxed);
    let owner = if schema.trim().is_empty() {
        let (_, rows) = crate::query_all(b.as_mut(), crate::dialect::current_schema_sql(db))?;
        rows.into_iter().next().and_then(|r| r.into_iter().next().flatten()).unwrap_or_default()
    } else {
        crate::dialect::fold(db, schema.trim())
    };
    let Some((cq, fq)) = queries(db, &owner) else { anyhow::bail!("ER diagrams are not available on this database.") };
    if stopped() {
        return Ok(None);
    }
    step(&format!("Reading the columns of {owner}…"));
    let mut t = tables(&crate::query_all(b.as_mut(), &cq)?.1);
    if !pattern.trim().is_empty() {
        t.retain(|n, _| crate::config::name_matches(pattern, n));
    }
    if t.is_empty() || stopped() {
        return Ok(None);
    }
    step(&format!("Reading the foreign keys of {owner} ({} tables)…", t.len()));
    let fks: Vec<(String, String)> = crate::query_all(b.as_mut(), &fq)?.1.into_iter().filter_map(|r| Some((r.first().cloned().flatten()?, r.get(1).cloned().flatten()?))).collect();
    *cancel.lock().unwrap() = None;
    if stopped() {
        return Ok(None);
    }
    step(&format!("Drawing {} tables…", t.len()));
    let n = t.len();
    Ok(Some((owner.clone(), svg(&format!("{owner} — {n} tables"), &t, &fks), n)))
}

/// One table in the diagram.
#[derive(Default, Debug)]
pub struct Table {
    /// (column, type, in the primary key).
    pub cols: Vec<(String, String, bool)>,
}

/// Tables from the columns query rows, kept in name order.
pub fn tables(rows: &[Vec<Option<String>>]) -> BTreeMap<String, Table> {
    let mut t: BTreeMap<String, Table> = BTreeMap::new();
    for r in rows {
        let g = |i: usize| r.get(i).cloned().flatten().unwrap_or_default();
        t.entry(g(0)).or_default().cols.push((g(1), g(2).to_lowercase(), g(3) == "Y"));
    }
    t
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

const W: i32 = 230;
const ROW: i32 = 16;
const HEAD: i32 = 24;
const GAP: i32 = 70;
/// Columns listed per box; the rest is summarised.
const MAX_COLS: usize = 25;

/// SVG of `tables` (grid layout, square-ish) and foreign keys `fks` (table → referenced),
/// drawn as arrows between box edges.
pub fn svg(title: &str, tables: &BTreeMap<String, Table>, fks: &[(String, String)]) -> String {
    let n = tables.len().max(1);
    let per_row = (n as f64).sqrt().ceil() as usize;
    let height = |t: &Table| HEAD + ROW * (t.cols.len().min(MAX_COLS) as i32 + i32::from(t.cols.len() > MAX_COLS)) + 6;
    // Row heights: the tallest box of each grid row.
    let names: Vec<&String> = tables.keys().collect();
    let rows: Vec<i32> = names.chunks(per_row).map(|c| c.iter().map(|n| height(&tables[*n])).max().unwrap_or(0)).collect();
    let mut pos: BTreeMap<&str, (i32, i32, i32)> = BTreeMap::new();
    let mut y = 40;
    for (ri, chunk) in names.chunks(per_row).enumerate() {
        for (ci, name) in chunk.iter().enumerate() {
            pos.insert(name.as_str(), (20 + ci as i32 * (W + GAP), y, height(&tables[*name])));
        }
        y += rows[ri] + GAP;
    }
    let (w, h) = (20 + per_row as i32 * (W + GAP), y);
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" font-family=\"Menlo, Consolas, monospace\" font-size=\"12\">\n<defs><marker id=\"a\" viewBox=\"0 0 10 10\" refX=\"10\" refY=\"5\" markerWidth=\"8\" markerHeight=\"8\" orient=\"auto-start-reverse\"><path d=\"M0 0L10 5L0 10z\" fill=\"#4a6d8c\"/></marker></defs>\n<rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>\n<text x=\"20\" y=\"24\" font-size=\"15\" font-weight=\"bold\">{}</text>\n",
        esc(title)
    );
    // Lines first, under the boxes: from the referencing box to the referenced one.
    for (from, to) in fks {
        let (Some(a), Some(b)) = (pos.get(from.as_str()), pos.get(to.as_str())) else { continue };
        let (ax, ay, bx, by) = (a.0 + W / 2, a.1 + a.2 / 2, b.0 + W / 2, b.1 + b.2 / 2);
        // Leave / enter at the box side facing the other box.
        let (x1, x2) = if bx > ax { (a.0 + W, b.0) } else if bx < ax { (a.0, b.0 + W) } else { (ax, bx) };
        let (y1, y2) = if bx == ax { if by > ay { (a.1 + a.2, b.1) } else { (a.1, b.1 + b.2) } } else { (ay, by) };
        out += &format!("<line x1=\"{x1}\" y1=\"{y1}\" x2=\"{x2}\" y2=\"{y2}\" stroke=\"#4a6d8c\" stroke-width=\"1.4\" marker-end=\"url(#a)\"/>\n");
    }
    for (name, t) in tables {
        let (x, y, hh) = pos[name.as_str()];
        out += &format!("<g><rect x=\"{x}\" y=\"{y}\" width=\"{W}\" height=\"{hh}\" rx=\"4\" fill=\"#f7fafd\" stroke=\"#4a6d8c\"/><rect x=\"{x}\" y=\"{y}\" width=\"{W}\" height=\"{HEAD}\" rx=\"4\" fill=\"#dce8f3\" stroke=\"#4a6d8c\"/><text x=\"{}\" y=\"{}\" font-weight=\"bold\">{}</text>\n", x + 8, y + 16, esc(name));
        for (i, (c, ty, pk)) in t.cols.iter().take(MAX_COLS).enumerate() {
            let ty: String = ty.chars().take(14).collect();
            let cy = y + HEAD + 13 + ROW * i as i32;
            out += &format!("<text x=\"{}\" y=\"{cy}\"{}>{}{}</text><text x=\"{}\" y=\"{cy}\" fill=\"#7a8a99\" text-anchor=\"end\">{}</text>\n", x + 8, if *pk { " font-weight=\"bold\"" } else { "" }, if *pk { "🔑 " } else { "" }, esc(c), x + W - 8, esc(&ty));
        }
        if t.cols.len() > MAX_COLS {
            out += &format!("<text x=\"{}\" y=\"{}\" fill=\"#7a8a99\">… {} more columns</text>\n", x + 8, y + HEAD + 13 + ROW * MAX_COLS as i32, t.cols.len() - MAX_COLS);
        }
        out += "</g>\n";
    }
    out + "</svg>\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole job on a connection of its own (SQLite file): schema, columns, keys, SVG.
    #[test]
    fn generate_on_own_connection() {
        let dir = std::env::temp_dir().join(format!("rsqldev-er-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("er.db");
        let _ = std::fs::remove_file(&file);
        let cfg = crate::config::ConnConfig { name: "er".into(), driver: "sqlite".into(), database: file.display().to_string(), ..Default::default() };
        let prefs = crate::config::Prefs::default();
        {
            let mut b = crate::backend::connect(&cfg, "", &prefs).unwrap();
            for s in ["CREATE TABLE dept (id INTEGER PRIMARY KEY, name TEXT)", "CREATE TABLE emp (id INTEGER PRIMARY KEY, dept_id INTEGER REFERENCES dept(id))", "CREATE TABLE zzz (x INTEGER)"] {
                crate::query_all(b.as_mut(), s).unwrap();
            }
            b.commit().unwrap();
        }
        let (cancel, stop) = (std::sync::Mutex::new(None), std::sync::atomic::AtomicBool::new(false));
        let steps = std::cell::RefCell::new(vec![]);
        let step = |s: &str| steps.borrow_mut().push(s.to_string());
        let (owner, svg, n) = generate(&cfg, "", &prefs, "", "%e%", &cancel, &stop, &step).unwrap().unwrap();
        assert_eq!((owner.as_str(), n), ("main", 2));
        assert!(svg.contains(">dept<") && svg.contains(">emp<") && !svg.contains(">zzz<") && svg.contains("<line"));
        assert!(steps.borrow().first().is_some_and(|s| s.starts_with("Connecting")));
        // Stopped before reading: nothing drawn.
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(generate(&cfg, "", &prefs, "", "", &cancel, &stop, &step).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn diagram() {
        let r = |v: &[&str]| v.iter().map(|s| (!s.is_empty()).then(|| s.to_string())).collect::<Vec<_>>();
        let t = tables(&[r(&["DEPT", "ID", "NUMBER", "Y"]), r(&["EMP", "ID", "NUMBER", "Y"]), r(&["EMP", "DEPT_ID", "NUMBER", ""]), r(&["EMP", "NOTE", "VARCHAR2", ""])]);
        assert_eq!(t["EMP"].cols.len(), 3);
        let s = svg("HR & co", &t, &[("EMP".into(), "DEPT".into()), ("EMP".into(), "GONE".into())]);
        assert!(s.starts_with("<svg") && s.ends_with("</svg>\n"));
        assert!(s.contains(">DEPT</text>") && s.contains("HR &amp; co"));
        assert_eq!(s.matches("<line").count(), 1, "a key to a table outside the diagram is skipped");
        for db in [Db::Oracle, Db::Postgres, Db::Mysql, Db::Mssql, Db::Sqlite, Db::Duckdb] {
            let (a, b) = queries(db, "O'X").unwrap();
            assert!(!a.contains('{') && !b.contains('{'), "{db:?}");
        }
        assert!(queries(Db::Clickhouse, "x").is_none());
    }
}
