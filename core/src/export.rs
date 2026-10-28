//! Result set export / copy formats. [`Export`] streams to a file batch by batch, so exporting a
//! huge result keeps memory flat; the string helpers serve tests and small in-memory uses.

use crate::registry::Kind as Db;
use crate::{Cell, Column, Result, lob};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Format {
    /// Delimited text (comma by default, see [`Options::delimiter`]).
    Csv,
    Xlsx,
    Json,
    Inserts,
    Html,
    Xml,
}

/// Export wizard options.
#[derive(Clone, Debug)]
pub struct Options {
    /// Column names first (CSV, HTML).
    pub header: bool,
    /// CSV / delimited separator.
    pub delimiter: char,
    /// Table named in INSERT statements.
    pub table: String,
}

impl Default for Options {
    fn default() -> Self {
        Options { header: true, delimiter: ',', table: "EXPORT_TABLE".into() }
    }
}

fn csv_field(s: &str, delim: char) -> String {
    if s.contains([delim, '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

fn xml_esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Tab separated, for the clipboard (pastes into spreadsheets). Spilled LOBs copy as their label.
pub fn tsv(cols: Option<&[Column]>, rows: &[Vec<Cell>]) -> String {
    let clean = |s: &str| lob::display(s).replace(['\t', '\n', '\r'], " ");
    let mut lines = vec![];
    if let Some(c) = cols {
        lines.push(c.iter().map(|c| clean(&c.name)).collect::<Vec<_>>().join("\t"));
    }
    for r in rows {
        lines.push(r.iter().map(|v| v.as_deref().map(clean).unwrap_or_default()).collect::<Vec<_>>().join("\t"));
    }
    lines.join("\n")
}

/// Markdown table (Copy As Markdown): pipes escaped, line breaks flattened.
pub fn markdown(cols: &[Column], rows: &[Vec<Cell>]) -> String {
    let cell = |s: &str| lob::display(s).replace('|', "\\|").replace(['\n', '\r'], " ");
    let mut out = format!("| {} |\n|{}|\n", cols.iter().map(|c| cell(&c.name)).collect::<Vec<_>>().join(" | "), cols.iter().map(|c| if c.numeric { " ---: " } else { " --- " }).collect::<Vec<_>>().join("|"));
    for r in rows {
        out += &format!("| {} |\n", r.iter().map(|v| v.as_deref().map(cell).unwrap_or_default()).collect::<Vec<_>>().join(" | "));
    }
    out
}

/// `(1, 2, 3)` / `('a', 'b')` from the first selected column, for pasting into `WHERE x IN`.
/// Distinct values in order; NULLs skipped.
pub fn in_list(db: Db, col: &Column, rows: &[Vec<Cell>]) -> String {
    let mut seen = std::collections::HashSet::new();
    let items: Vec<String> = rows.iter().filter_map(|r| r.first().cloned().flatten()).filter(|v| seen.insert(v.clone())).map(|v| crate::dml::literal(db, col, &Some(v))).collect();
    format!("({})", items.join(", "))
}

fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for ch in s.chars() {
        match ch {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Spilled LOB cells expand to their full value in exports.
fn full(row: &[Cell]) -> Vec<Cell> {
    row.iter().map(|v| v.as_deref().map(|s| lob::expand(s).into_owned())).collect()
}

/// Text formats, written piece by piece.
fn begin(w: &mut dyn Write, fmt: Format, cols: &[Column], o: &Options) -> std::io::Result<()> {
    let d = o.delimiter.to_string();
    match fmt {
        Format::Csv if !o.header => Ok(()),
        Format::Csv => writeln!(w, "{}", cols.iter().map(|c| csv_field(&c.name, o.delimiter)).collect::<Vec<_>>().join(&d)),
        Format::Html => {
            write!(w, "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><title>Export</title></head><body>\n<table border=\"1\">\n")?;
            if o.header {
                writeln!(w, "<tr>{}</tr>", cols.iter().map(|c| format!("<th>{}</th>", xml_esc(&c.name))).collect::<String>())?;
            }
            Ok(())
        }
        Format::Xml => write!(w, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<RESULTS>\n"),
        Format::Json => write!(
            w,
            "{{\"results\":[{{\"columns\":[{}],\"items\":[\n",
            cols.iter().map(|c| format!("{{\"name\":{},\"type\":{}}}", json_str(&c.name), json_str(&c.type_name))).collect::<Vec<_>>().join(",")
        ),
        Format::Inserts | Format::Xlsx => Ok(()),
    }
}

fn row(w: &mut dyn Write, fmt: Format, cols: &[Column], db: Db, o: &Options, n: u64, r: &[Cell]) -> std::io::Result<()> {
    let r = full(r);
    let table = o.table.as_str();
    match fmt {
        Format::Csv => writeln!(w, "{}", r.iter().map(|v| v.as_deref().map(|s| csv_field(s, o.delimiter)).unwrap_or_default()).collect::<Vec<_>>().join(&o.delimiter.to_string())),
        Format::Html => writeln!(w, "<tr>{}</tr>", r.iter().map(|v| format!("<td>{}</td>", v.as_deref().map(xml_esc).unwrap_or_default())).collect::<String>()),
        Format::Xml => writeln!(
            w,
            "  <ROW>\n{}  </ROW>",
            cols.iter().zip(&r).map(|(c, v)| format!("    <COLUMN NAME=\"{}\">{}</COLUMN>\n", xml_esc(&c.name), v.as_deref().map(xml_esc).unwrap_or_default())).collect::<String>()
        ),
        Format::Json => {
            let f: Vec<String> = cols
                .iter()
                .zip(&r)
                .map(|(c, v)| {
                    let val = match v {
                        None => "null".to_string(),
                        Some(s) if c.numeric && s.parse::<f64>().is_ok_and(f64::is_finite) => s.clone(),
                        Some(s) => json_str(s),
                    };
                    format!("{}:{val}", json_str(&c.name))
                })
                .collect();
            write!(w, "{}{{{}}}", if n == 0 { "" } else { ",\n" }, f.join(","))
        }
        Format::Inserts => writeln!(w, "{};", crate::dml::insert(db, table, cols, None, &r, &[])),
        Format::Xlsx => Ok(()),
    }
}

fn end(w: &mut dyn Write, fmt: Format) -> std::io::Result<()> {
    match fmt {
        Format::Json => w.write_all(b"\n]}]}\n"),
        Format::Html => w.write_all(b"</table>\n</body></html>\n"),
        Format::Xml => w.write_all(b"</RESULTS>\n"),
        _ => Ok(()),
    }
}

enum Out {
    Text(std::io::BufWriter<std::fs::File>),
    /// Constant-memory worksheet: rows are flushed to a temp file as they are written.
    Xlsx(Box<rust_xlsxwriter::Workbook>, PathBuf),
}

/// A file export in progress: `rows` per fetched batch, then `finish`.
pub struct Export {
    out: Out,
    fmt: Format,
    cols: Vec<Column>,
    db: Db,
    opts: Options,
    n: u64,
}

impl Export {
    pub fn create(path: &Path, fmt: Format, cols: &[Column], db: Db, table: &str) -> Result<Export> {
        Export::create_with(path, fmt, cols, db, &Options { table: table.into(), ..Options::default() })
    }

    pub fn create_with(path: &Path, fmt: Format, cols: &[Column], db: Db, opts: &Options) -> Result<Export> {
        let out = if fmt == Format::Xlsx {
            let mut wb = rust_xlsxwriter::Workbook::new();
            let ws = wb.add_worksheet_with_constant_memory()?;
            let bold = rust_xlsxwriter::Format::new().set_bold();
            for (c, col) in cols.iter().enumerate() {
                ws.write_string_with_format(0, c as u16, &col.name, &bold)?;
                ws.set_column_width(c as u16, (col.name.chars().count() + 4).clamp(10, 40) as f64)?;
            }
            ws.set_freeze_panes(1, 0)?;
            Out::Xlsx(Box::new(wb), path.to_path_buf())
        } else {
            let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
            begin(&mut w, fmt, cols, opts)?;
            Out::Text(w)
        };
        Ok(Export { out, fmt, cols: cols.to_vec(), db, opts: opts.clone(), n: 0 })
    }

    pub fn rows(&mut self, rows: &[Vec<Cell>]) -> Result<()> {
        match &mut self.out {
            Out::Text(w) => {
                for r in rows {
                    row(w, self.fmt, &self.cols, self.db, &self.opts, self.n, r)?;
                    self.n += 1;
                }
            }
            Out::Xlsx(wb, _) => {
                let ws = wb.worksheet_from_index(0)?;
                for r in rows {
                    // Excel allows 1,048,576 rows per sheet.
                    if self.n >= 1_048_575 {
                        anyhow::bail!("Excel sheets hold at most 1,048,575 data rows: export to CSV instead");
                    }
                    let x = self.n as u32 + 1;
                    for (c, v) in full(r).iter().enumerate() {
                        let Some(s) = v else { continue };
                        match s.parse::<f64>() {
                            Ok(n) if self.cols[c].numeric && n.is_finite() => ws.write_number(x, c as u16, n)?,
                            // Excel caps a cell at 32767 characters.
                            _ => ws.write_string(x, c as u16, s.chars().take(32767).collect::<String>())?,
                        };
                    }
                    self.n += 1;
                }
            }
        }
        Ok(())
    }

    /// Rows written so far.
    pub fn count(&self) -> u64 {
        self.n
    }

    pub fn finish(self) -> Result<u64> {
        match self.out {
            Out::Text(mut w) => {
                end(&mut w, self.fmt)?;
                w.flush()?;
            }
            Out::Xlsx(mut wb, path) => wb.save(path)?,
        }
        Ok(self.n)
    }
}

/// Whole export as a string (clipboard, tests). Not for Xlsx.
pub fn text_with(fmt: Format, db: Db, opts: &Options, cols: &[Column], rows: &[Vec<Cell>]) -> String {
    let mut v = vec![];
    let _ = begin(&mut v, fmt, cols, opts);
    for (n, r) in rows.iter().enumerate() {
        let _ = row(&mut v, fmt, cols, db, opts, n as u64, r);
    }
    let _ = end(&mut v, fmt);
    String::from_utf8_lossy(&v).into_owned()
}

pub fn csv(cols: &[Column], rows: &[Vec<Cell>]) -> String {
    text_with(Format::Csv, Db::Sqlite, &Options::default(), cols, rows)
}

pub fn json(cols: &[Column], rows: &[Vec<Cell>]) -> String {
    text_with(Format::Json, Db::Sqlite, &Options::default(), cols, rows)
}

pub fn inserts(db: Db, table: &str, cols: &[Column], rows: &[Vec<Cell>]) -> String {
    text_with(Format::Inserts, db, &Options { table: table.into(), ..Options::default() }, cols, rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        let cols = vec![Column { name: "ID".into(), type_name: "NUMBER".into(), numeric: true }, Column { name: "N".into(), type_name: "VARCHAR2".into(), numeric: false }];
        let rows = vec![vec![Some("1".into()), Some("a,\"b\"".into())], vec![None, None]];
        assert_eq!(csv(&cols, &rows), "ID,N\n1,\"a,\"\"b\"\"\"\n,\n");
        assert!(json(&cols, &rows).contains("{\"ID\":1,\"N\":\"a,\\\"b\\\"\"},\n{\"ID\":null,\"N\":null}\n]}]}"));
        assert_eq!(tsv(None, &rows), "1\ta,\"b\"\n\t");
        assert_eq!(inserts(Db::Postgres, "t", &cols, &rows[..1]), "INSERT INTO t (\"ID\", \"N\") VALUES (1, 'a,\"b\"');\n");
    }

    #[test]
    fn copy_as() {
        let cols = vec![Column { name: "ID".into(), type_name: "NUMBER".into(), numeric: true }, Column { name: "N".into(), type_name: "VARCHAR2".into(), numeric: false }];
        let rows = vec![vec![Some("1".into()), Some("a|b".into())], vec![Some("2".into()), None]];
        assert_eq!(markdown(&cols, &rows), "| ID | N |\n| ---: | --- |\n| 1 | a\\|b |\n| 2 |  |\n");
        let names = vec![vec![Some("O'Neil".into())], vec![None], vec![Some("x".into())], vec![Some("O'Neil".into())]];
        assert_eq!(in_list(Db::Oracle, &cols[1], &names), "('O''Neil', 'x')");
        assert_eq!(in_list(Db::Oracle, &cols[0], &[vec![Some("1".into())], vec![Some("2".into())]]), "(1, 2)");
    }

    #[test]
    fn options_and_markup() {
        let cols = vec![Column { name: "ID".into(), type_name: "NUMBER".into(), numeric: true }, Column { name: "N".into(), type_name: "".into(), numeric: false }];
        let rows = vec![vec![Some("1".into()), Some("a;<b>".into())], vec![None, None]];
        let o = Options { header: false, delimiter: ';', ..Options::default() };
        assert_eq!(text_with(Format::Csv, Db::Sqlite, &o, &cols, &rows), "1;\"a;<b>\"\n;\n");
        let h = text_with(Format::Html, Db::Sqlite, &Options::default(), &cols, &rows[..1]);
        assert!(h.contains("<tr><th>ID</th><th>N</th></tr>\n<tr><td>1</td><td>a;&lt;b&gt;</td></tr>\n</table>"), "{h}");
        let x = text_with(Format::Xml, Db::Sqlite, &Options::default(), &cols, &rows[..1]);
        assert!(x.contains("<ROW>\n    <COLUMN NAME=\"ID\">1</COLUMN>\n    <COLUMN NAME=\"N\">a;&lt;b&gt;</COLUMN>\n  </ROW>\n</RESULTS>"), "{x}");
    }

    #[test]
    fn streamed_files() {
        let dir = std::env::temp_dir().join(format!("rsq-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cols = vec![Column { name: "ID".into(), type_name: "NUMBER".into(), numeric: true }, Column { name: "B".into(), type_name: "BLOB".into(), numeric: false }];
        let big: Vec<u8> = vec![7; 100_000];
        let spilled = lob::keep(format!("0x{}", crate::hex(&big)), true);
        let rows = vec![vec![Some("1".into()), Some(spilled.clone())], vec![Some("2".into()), None]];
        // Batches: as a grid drains a cursor.
        let p = dir.join("t.csv");
        let mut e = Export::create(&p, Format::Csv, &cols, Db::Sqlite, "T").unwrap();
        e.rows(&rows[..1]).unwrap();
        e.rows(&rows[1..]).unwrap();
        assert_eq!(e.finish().unwrap(), 2);
        let csv = std::fs::read_to_string(&p).unwrap();
        assert!(csv.starts_with("ID,B\n1,0x0707") && csv.ends_with("\n2,\n"), "spilled BLOB exported in full");
        let p = dir.join("t.xlsx");
        let mut e = Export::create(&p, Format::Xlsx, &cols, Db::Sqlite, "T").unwrap();
        e.rows(&rows[1..]).unwrap();
        e.finish().unwrap();
        let (h, r) = crate::import::read(&p).unwrap();
        assert_eq!(h, ["ID", "B"]);
        assert_eq!(r, vec![vec![Some("2".into()), None]]);
        lob::release(&spilled);
        let _ = std::fs::remove_dir_all(dir);
    }
}
