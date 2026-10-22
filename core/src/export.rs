//! Result set export / copy formats.

use crate::registry::Kind as Db;
use crate::{Cell, Column};

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

pub fn csv(cols: &[Column], rows: &[Vec<Cell>]) -> String {
    let mut out = cols.iter().map(|c| csv_field(&c.name)).collect::<Vec<_>>().join(",");
    for r in rows {
        out.push('\n');
        out.push_str(&r.iter().map(|v| v.as_deref().map(csv_field).unwrap_or_default()).collect::<Vec<_>>().join(","));
    }
    out.push('\n');
    out
}

/// Tab separated, for the clipboard (pastes into spreadsheets).
pub fn tsv(cols: Option<&[Column]>, rows: &[Vec<Cell>]) -> String {
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    let mut lines = vec![];
    if let Some(c) = cols {
        lines.push(c.iter().map(|c| clean(&c.name)).collect::<Vec<_>>().join("\t"));
    }
    for r in rows {
        lines.push(r.iter().map(|v| v.as_deref().map(clean).unwrap_or_default()).collect::<Vec<_>>().join("\t"));
    }
    lines.join("\n")
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

pub fn json(cols: &[Column], rows: &[Vec<Cell>]) -> String {
    let items: Vec<String> = rows
        .iter()
        .map(|r| {
            let f: Vec<String> = cols
                .iter()
                .zip(r)
                .map(|(c, v)| {
                    let val = match v {
                        None => "null".to_string(),
                        Some(s) if c.numeric && s.parse::<f64>().is_ok_and(f64::is_finite) => s.clone(),
                        Some(s) => json_str(s),
                    };
                    format!("{}:{val}", json_str(&c.name))
                })
                .collect();
            format!("{{{}}}", f.join(","))
        })
        .collect();
    format!("{{\"results\":[{{\"columns\":[{}],\"items\":[\n{}\n]}}]}}\n", cols.iter().map(|c| format!("{{\"name\":{},\"type\":{}}}", json_str(&c.name), json_str(&c.type_name))).collect::<Vec<_>>().join(","), items.join(",\n"))
}

pub fn inserts(db: Db, table: &str, cols: &[Column], rows: &[Vec<Cell>]) -> String {
    rows.iter().map(|r| crate::dml::insert(db, table, cols, None, r, &[]) + ";\n").collect()
}

/// Excel workbook: bold frozen header row, numeric columns written as numbers.
pub fn xlsx(cols: &[Column], rows: &[Vec<Cell>]) -> crate::Result<Vec<u8>> {
    use rust_xlsxwriter::{Format, Workbook};
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet();
    let bold = Format::new().set_bold();
    for (c, col) in cols.iter().enumerate() {
        ws.write_string_with_format(0, c as u16, &col.name, &bold)?;
    }
    for (r, row) in rows.iter().enumerate() {
        let r = r as u32 + 1;
        for (c, v) in row.iter().enumerate() {
            let Some(s) = v else { continue };
            match s.parse::<f64>() {
                Ok(n) if cols[c].numeric && n.is_finite() => ws.write_number(r, c as u16, n)?,
                // Excel caps a cell at 32767 characters.
                _ => ws.write_string(r, c as u16, s.chars().take(32767).collect::<String>())?,
            };
        }
    }
    ws.set_freeze_panes(1, 0)?;
    ws.autofit();
    Ok(wb.save_to_buffer()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        let cols = vec![Column { name: "ID".into(), type_name: "NUMBER".into(), numeric: true }, Column { name: "N".into(), type_name: "VARCHAR2".into(), numeric: false }];
        let rows = vec![vec![Some("1".into()), Some("a,\"b\"".into())], vec![None, None]];
        assert_eq!(csv(&cols, &rows), "ID,N\n1,\"a,\"\"b\"\"\"\n,\n");
        assert!(json(&cols, &rows).contains("{\"ID\":1,\"N\":\"a,\\\"b\\\"\"},\n{\"ID\":null,\"N\":null}"));
        assert_eq!(tsv(None, &rows), "1\ta,\"b\"\n\t");
        assert_eq!(inserts(Db::Postgres, "t", &cols, &rows[..1]), "INSERT INTO t (\"ID\", \"N\") VALUES (1, 'a,\"b\"');\n");
        let x = xlsx(&cols, &rows).unwrap();
        assert!(x.starts_with(b"PK"), "xlsx is a zip");
        let (h, r) = crate::import::read_xlsx(std::io::Cursor::new(x)).unwrap();
        assert_eq!(h, ["ID", "N"]);
        assert_eq!(r, vec![vec![Some("1".into()), Some("a,\"b\"".into())]], "an all-NULL row writes no cells, so it is not read back");
    }
}
