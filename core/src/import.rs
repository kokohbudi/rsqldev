//! Data import: read CSV or a spreadsheet (first sheet) into a header row and text cells,
//! then match headers to a table's columns.

use crate::{Cell, Column, Result};
use anyhow::{Context, bail};
use std::path::Path;

/// Header names and rows. Empty fields are NULL, like SQL Developer's import.
pub fn read(path: &Path) -> Result<(Vec<String>, Vec<Vec<Cell>>)> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "xlsx" | "xlsm" | "xls" | "xlsb" | "ods" => {
            let mut wb = calamine::open_workbook_auto(path).with_context(|| format!("open {}", path.display()))?;
            sheet(&mut wb)
        }
        _ => read_csv(std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?),
    }
}

pub fn read_csv(r: impl std::io::Read) -> Result<(Vec<String>, Vec<Vec<Cell>>)> {
    // Tolerate a UTF-8 BOM (Excel's "CSV UTF-8").
    let mut data = vec![];
    let mut r = r;
    r.read_to_end(&mut data)?;
    let data = data.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&data);
    let delim = sniff(data);
    let mut rd = csv::ReaderBuilder::new().delimiter(delim).flexible(true).from_reader(data);
    let headers = rd.headers()?.iter().map(|h| h.trim().to_string()).collect();
    let mut rows = vec![];
    for rec in rd.records() {
        rows.push(rec?.iter().map(|f| (!f.is_empty()).then(|| f.to_string())).collect());
    }
    Ok((headers, rows))
}

/// Comma unless the header line has more semicolons or tabs (European Excel exports).
fn sniff(data: &[u8]) -> u8 {
    let line = data.split(|b| *b == b'\n').next().unwrap_or(data);
    [b',', b';', b'\t'].into_iter().max_by_key(|d| line.iter().filter(|b| *b == d).count()).unwrap_or(b',')
}

pub fn read_xlsx<R: std::io::Read + std::io::Seek>(r: R) -> Result<(Vec<String>, Vec<Vec<Cell>>)> {
    let mut wb: calamine::Xlsx<R> = calamine::Reader::new(r)?;
    sheet(&mut wb)
}

fn sheet<R: std::io::Read + std::io::Seek, W: calamine::Reader<R>>(wb: &mut W) -> Result<(Vec<String>, Vec<Vec<Cell>>)>
where
    W::Error: std::error::Error + Send + Sync + 'static,
{
    let Some(name) = wb.sheet_names().first().cloned() else { bail!("the workbook has no sheets") };
    let range = wb.worksheet_range(&name)?;
    let mut it = range.rows();
    let Some(head) = it.next() else { bail!("sheet '{name}' is empty") };
    let headers = head.iter().map(|c| text(c).unwrap_or_default().trim().to_string()).collect();
    Ok((headers, it.map(|r| r.iter().map(text).collect()).collect()))
}

/// Spreadsheet cell as text: whole numbers without `.0`, dates as ISO (`YYYY-MM-DD[ HH:MM:SS]`).
fn text(c: &calamine::Data) -> Cell {
    use calamine::{Data, DataType};
    match c {
        Data::Empty | Data::Error(_) => None,
        Data::Float(f) if f.fract() == 0.0 && f.abs() < 1e15 => Some(format!("{}", *f as i64)),
        Data::DateTime(_) | Data::DateTimeIso(_) => c.as_datetime().map(|d| {
            if d.time() == chrono::NaiveTime::MIN { d.format("%Y-%m-%d").to_string() } else { d.format("%Y-%m-%d %H:%M:%S").to_string() }
        }),
        other => Some(other.to_string()),
    }
}

/// For each file column, the table column it fills (same name, ignoring case and quotes).
pub fn map_columns(headers: &[String], cols: &[Column]) -> Vec<Option<usize>> {
    let norm = |s: &str| s.trim().trim_matches(['"', '`', '[', ']']).to_lowercase();
    headers.iter().map(|h| cols.iter().position(|c| norm(&c.name) == norm(h))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_and_mapping() {
        let (h, r) = read_csv("\u{feff}id;Name;extra\n1;O'Neil;\n2;\"a;b\";x\n".as_bytes()).unwrap();
        assert_eq!(h, ["id", "Name", "extra"]);
        assert_eq!(r[0], vec![Some("1".into()), Some("O'Neil".into()), None]);
        assert_eq!(r[1][1].as_deref(), Some("a;b"));
        let cols = vec![Column { name: "NAME".into(), type_name: "VARCHAR2".into(), numeric: false }, Column { name: "ID".into(), type_name: "NUMBER".into(), numeric: true }];
        assert_eq!(map_columns(&h, &cols), vec![Some(1), Some(0), None]);
    }
}
