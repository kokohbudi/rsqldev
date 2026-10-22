//! Worksheet script handling: split text into statements (SQL, PL/SQL blocks, SQL*Plus commands),
//! substitution variables, and the SQL*Plus command subset the script runner understands.

use crate::registry::Kind as Db;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Sql,
    /// PL/SQL or T-SQL module body: sent verbatim, terminated by `/` or `GO`.
    Block,
    /// SQL*Plus / client command (SET, DEFINE, @file, `/`, ...).
    Command,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub kind: Kind,
}

/// SQL*Plus commands with their minimum abbreviation length.
const COMMANDS: &[(&str, usize)] = &[
    ("SET", 3),
    ("SHOW", 3),
    ("DEFINE", 3),
    ("UNDEFINE", 5),
    ("PROMPT", 3),
    ("SPOOL", 3),
    ("REMARK", 3),
    ("START", 3),
    ("EXECUTE", 4),
    ("DESCRIBE", 4),
    ("CLEAR", 2),
    ("CONNECT", 4),
    ("VARIABLE", 3),
    ("PRINT", 5),
    ("WHENEVER", 8),
    ("PAUSE", 3),
    ("ACCEPT", 3),
    ("COLUMN", 3),
    ("TTITLE", 3),
    ("BTITLE", 3),
    ("BREAK", 3),
    ("COMPUTE", 4),
];

/// Canonical SQL*Plus command name for the first word of a line, if it is one.
pub fn command_name(line: &str) -> Option<&'static str> {
    let t = line.trim_start();
    if t.starts_with('@') {
        return Some("@");
    }
    let mut words = t.split_whitespace();
    let w = words.next()?.trim_end_matches(';').to_uppercase();
    let cmd = COMMANDS.iter().find(|(c, min)| w.len() >= *min && c.starts_with(&w)).map(|(c, _)| *c)?;
    let next = words.next().unwrap_or("").to_uppercase();
    // SQL statements that share a SQL*Plus keyword.
    if cmd == "SET" && ["TRANSACTION", "ROLE", "CONSTRAINT", "CONSTRAINTS"].contains(&next.as_str()) {
        return None;
    }
    if cmd == "DESCRIBE" && w.len() < 4 {
        return None;
    }
    Some(cmd)
}

fn is_block(head: &str, db: Db) -> bool {
    let words: Vec<String> = head.split_whitespace().take(6).map(|w| w.to_uppercase()).collect();
    let w = |i: usize| words.get(i).map(String::as_str).unwrap_or("");
    match db {
        Db::Oracle => {
            if matches!(w(0), "DECLARE" | "BEGIN") {
                return true;
            }
            if w(0) != "CREATE" {
                return false;
            }
            let mut i = 1;
            if w(1) == "OR" && w(2) == "REPLACE" {
                i = 3;
            }
            if matches!(w(i), "EDITIONABLE" | "NONEDITIONABLE" | "EDITIONING") {
                i += 1;
            }
            matches!(w(i), "FUNCTION" | "PROCEDURE" | "PACKAGE" | "TRIGGER" | "TYPE" | "LIBRARY" | "JAVA")
        }
        Db::Mssql => {
            matches!(w(0), "CREATE" | "ALTER" | "CREATE OR ALTER")
                && (matches!(w(1), "PROCEDURE" | "PROC" | "FUNCTION" | "TRIGGER" | "VIEW")
                    || (w(1) == "OR" && matches!(w(3), "PROCEDURE" | "PROC" | "FUNCTION" | "TRIGGER" | "VIEW")))
        }
        _ => false,
    }
}

/// Line terminating a block / batch: `/` (Oracle) or `GO` (SQL Server).
fn is_terminator_line(line: &str, db: Db) -> bool {
    let t = line.trim();
    (db == Db::Oracle && t == "/") || (db == Db::Mssql && t.eq_ignore_ascii_case("go"))
}

fn line_at(s: &str, i: usize) -> &str {
    let e = s[i..].find('\n').map_or(s.len(), |n| i + n);
    &s[i..e]
}

fn next_line(s: &str, i: usize) -> usize {
    s[i..].find('\n').map_or(s.len(), |n| i + n + 1)
}

/// Skip whitespace and comments.
fn skip_trivia(s: &str, mut i: usize, db: Db) -> usize {
    let b = s.as_bytes();
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if s[i..].starts_with("--") || (db == Db::Mysql && s[i..].starts_with('#')) {
            i = next_line(s, i);
        } else if s[i..].starts_with("/*") {
            i = s[i + 2..].find("*/").map_or(s.len(), |n| i + 2 + n + 2);
        } else {
            return i;
        }
    }
}

/// Split a worksheet/script into executable items.
pub fn split(s: &str, db: Db) -> Vec<Item> {
    let mut out = vec![];
    let mut delim = ";".to_string();
    let mut i = 0;
    loop {
        i = skip_trivia(s, i, db);
        if i >= s.len() {
            return out;
        }
        let line = line_at(s, i);
        if is_terminator_line(line, db) {
            if db == Db::Oracle {
                out.push(Item { start: i, end: i + line.len(), text: "/".into(), kind: Kind::Command });
            }
            i = next_line(s, i);
            continue;
        }
        if db == Db::Mysql && line.trim_start().to_uppercase().starts_with("DELIMITER ") {
            delim = line.trim()[10..].trim().to_string();
            i = next_line(s, i);
            continue;
        }
        if db == Db::Oracle && command_name(line).is_some() {
            // `-` at end of line continues the command.
            let start = i;
            let mut end = i + line.len();
            while s[start..end].trim_end().ends_with('-') && end < s.len() {
                end = end + 1 + line_at(s, end + 1).len();
            }
            let text = s[start..end].trim().trim_end_matches(';').trim().to_string();
            out.push(Item { start, end, text, kind: Kind::Command });
            i = end;
            continue;
        }
        let block = is_block(&s[i..s.len().min(i + 200)], db);
        let (end, next) = scan(s, i, db, block, &delim);
        let mut text = s[i..end].trim_end().to_string();
        if !block {
            text = text.trim_end_matches(delim.as_str()).trim_end().to_string();
        }
        if !text.is_empty() {
            out.push(Item { start: i, end, text, kind: if block { Kind::Block } else { Kind::Sql } });
        }
        i = next;
    }
}

/// Find the end of the statement starting at `i`. Returns (end of text, resume position).
fn scan(s: &str, i: usize, db: Db, block: bool, delim: &str) -> (usize, usize) {
    let b = s.as_bytes();
    let mut j = i;
    while j < b.len() {
        let rest = &s[j..];
        let c = b[j];
        if rest.starts_with("--") || (db == Db::Mysql && c == b'#') {
            j = next_line(s, j);
            if let Some(r) = line_end_terminates(s, j, db, block) {
                return (j.saturating_sub(1).max(i), r);
            }
            continue;
        }
        if rest.starts_with("/*") {
            j = s[j + 2..].find("*/").map_or(s.len(), |n| j + 2 + n + 2);
            continue;
        }
        // Oracle q'[...]' literals.
        if db == Db::Oracle && (c == b'q' || c == b'Q') && rest.len() > 3 && b[j + 1] == b'\'' {
            let open = rest[2..].chars().next().unwrap();
            let close = match open {
                '[' => ']',
                '(' => ')',
                '{' => '}',
                '<' => '>',
                o => o,
            };
            let pat = format!("{close}'");
            j = s[j + 2 + open.len_utf8()..].find(&pat).map_or(s.len(), |n| j + 2 + open.len_utf8() + n + pat.len());
            continue;
        }
        if c == b'\'' || c == b'"' || (db == Db::Mysql && c == b'`') {
            j += 1;
            while j < b.len() {
                if db == Db::Mysql && b[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if b[j] == c {
                    if b.get(j + 1) == Some(&c) {
                        j += 2;
                        continue;
                    }
                    break;
                }
                j += 1;
            }
            j += 1;
            continue;
        }
        if db == Db::Postgres && c == b'$' {
            let tag_len = rest[1..].find(|ch: char| !(ch.is_alphanumeric() || ch == '_')).map(|n| n + 1);
            if let Some(n) = tag_len.filter(|n| rest.as_bytes().get(*n) == Some(&b'$')) {
                let tag = &rest[..=n];
                j = s[j + tag.len()..].find(tag).map_or(s.len(), |m| j + tag.len() + m + tag.len());
                continue;
            }
        }
        if !block && rest.starts_with(delim) {
            return (j, j + delim.len());
        }
        if c == b'\n' {
            if let Some(r) = line_end_terminates(s, j + 1, db, block) {
                return (j, r);
            }
        }
        j += c.len_utf8_hint();
    }
    (s.len(), s.len())
}

/// Called at a line start: does this line end the current statement? Returns resume position.
fn line_end_terminates(s: &str, ls: usize, db: Db, block: bool) -> Option<usize> {
    if ls >= s.len() {
        return None;
    }
    let line = line_at(s, ls);
    if is_terminator_line(line, db) {
        return Some(next_line(s, ls));
    }
    // ponytail: blank line ends a plain statement (SQL Developer statement-at-cursor behaviour).
    if !block && line.trim().is_empty() {
        return Some(ls);
    }
    None
}

trait Utf8Hint {
    fn len_utf8_hint(self) -> usize;
}

impl Utf8Hint for u8 {
    fn len_utf8_hint(self) -> usize {
        match self {
            0x00..=0x7f => 1,
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            _ => 4,
        }
    }
}

/// Statement under the cursor (Ctrl/Cmd+Enter); falls back to the closest one before it.
pub fn statement_at(items: &[Item], pos: usize) -> Option<&Item> {
    items.iter().find(|it| it.start <= pos && pos <= it.end).or_else(|| items.iter().rev().find(|it| it.end <= pos)).or(items.first())
}

/// Substitution variables (`&name`, `&&name`), SQL*Plus style.
#[derive(Debug, Default, Clone)]
pub struct Defines {
    pub vars: BTreeMap<String, String>,
    pub disabled: bool,
}

impl Defines {
    /// Replace substitution variables; unknown ones are asked via `ask` (None = cancel run).
    pub fn substitute(&mut self, text: &str, ask: &mut dyn FnMut(&str) -> Option<String>) -> Option<String> {
        if self.disabled || !text.contains('&') {
            return Some(text.to_string());
        }
        let mut out = String::with_capacity(text.len());
        let mut it = text.char_indices().peekable();
        while let Some((_, c)) = it.next() {
            if c != '&' {
                out.push(c);
                continue;
            }
            let keep = it.peek().map(|x| x.1) == Some('&');
            if keep {
                it.next();
            }
            let mut name = String::new();
            while let Some(&(_, ch)) = it.peek() {
                if ch.is_alphanumeric() || ch == '_' || ch == '$' || ch == '#' {
                    name.push(ch);
                    it.next();
                } else {
                    break;
                }
            }
            if name.is_empty() {
                out.push('&');
                if keep {
                    out.push('&');
                }
                continue;
            }
            if it.peek().map(|x| x.1) == Some('.') {
                it.next(); // concatenation char
            }
            let key = name.to_uppercase();
            let v = match self.vars.get(&key) {
                Some(v) => v.clone(),
                None => {
                    let v = ask(&name)?;
                    if keep {
                        self.vars.insert(key, v.clone());
                    }
                    v
                }
            };
            out.push_str(&v);
        }
        Some(out)
    }
}

/// What the worksheet should do for a command item.
#[derive(Debug, PartialEq)]
pub enum Action {
    Run(String),
    Print(String),
    Output(bool),
    Describe(String),
    Spool(Option<String>),
    Include(String),
    Rerun,
    Clear,
    Nothing,
    Unsupported(String),
}

pub fn command(text: &str, defs: &mut Defines) -> Action {
    let t = text.trim();
    if t == "/" {
        return Action::Rerun;
    }
    if let Some(p) = t.strip_prefix("@@").or_else(|| t.strip_prefix('@')) {
        return Action::Include(p.trim().to_string());
    }
    let Some(cmd) = command_name(t) else { return Action::Unsupported(t.into()) };
    let rest = t.split_once(char::is_whitespace).map_or("", |x| x.1).trim();
    let up = rest.to_uppercase();
    let arg1 = up.split_whitespace().next().unwrap_or("");
    match cmd {
        "SET" => match arg1 {
            a if "SERVEROUTPUT".starts_with(a) && a.len() >= 9 => Action::Output(up.split_whitespace().nth(1) == Some("ON")),
            a if "DEFINE".starts_with(a) && a.len() >= 3 => {
                defs.disabled = up.split_whitespace().nth(1) == Some("OFF");
                Action::Nothing
            }
            _ => Action::Unsupported(t.into()),
        },
        "DEFINE" => {
            if let Some((k, v)) = rest.split_once('=') {
                defs.vars.insert(k.trim().to_uppercase(), v.trim().trim_matches('"').trim_matches('\'').to_string());
                Action::Nothing
            } else if rest.is_empty() {
                Action::Print(defs.vars.iter().map(|(k, v)| format!("DEFINE {k} = \"{v}\"")).collect::<Vec<_>>().join("\n"))
            } else {
                let k = rest.to_uppercase();
                Action::Print(defs.vars.get(&k).map_or(format!("SP2-0135: symbol {k} is UNDEFINED"), |v| format!("DEFINE {k} = \"{v}\"")))
            }
        }
        "UNDEFINE" => {
            for k in up.split_whitespace() {
                defs.vars.remove(k);
            }
            Action::Nothing
        }
        "PROMPT" => Action::Print(rest.into()),
        "REMARK" => Action::Nothing,
        "EXECUTE" => Action::Run(format!("BEGIN {}; END;", rest.trim_end_matches(';'))),
        "DESCRIBE" => Action::Describe(rest.into()),
        "SPOOL" => Action::Spool((arg1 != "OFF" && !rest.is_empty()).then(|| rest.to_string())),
        "START" => Action::Include(rest.into()),
        "CLEAR" if arg1.starts_with("SCR") => Action::Clear,
        "SHOW" if arg1 == "USER" => Action::Run("SELECT 'USER is \"' || USER || '\"' AS \"USER\" FROM DUAL".into()),
        "SHOW" if arg1.starts_with("ERR") => Action::Run(
            "SELECT name, type, line, position, text FROM user_errors ORDER BY name, type, sequence".into(),
        ),
        _ => Action::Unsupported(t.into()),
    }
}


/// SQL*Plus style text table for the Script Output pane.
pub fn format_table(cols: &[crate::Column], rows: &[Vec<crate::Cell>]) -> String {
    if cols.is_empty() {
        return String::new();
    }
    let cell = |v: &crate::Cell| v.as_deref().unwrap_or("").replace(['\n', '\r'], " ");
    let w: Vec<usize> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| rows.iter().map(|r| cell(&r[i]).chars().count()).chain([c.name.chars().count()]).max().unwrap_or(0).min(60))
        .collect();
    let pad = |s: String, i: usize| {
        let n = s.chars().count();
        let s: String = if n > w[i] { s.chars().take(w[i]).collect() } else { s };
        let fill = " ".repeat(w[i].saturating_sub(s.chars().count()));
        if cols[i].numeric { format!("{fill}{s}") } else { format!("{s}{fill}") }
    };
    let mut out = cols.iter().enumerate().map(|(i, c)| pad(c.name.clone(), i)).collect::<Vec<_>>().join(" ").trim_end().to_string();
    out.push('\n');
    out.push_str(&w.iter().map(|n| "-".repeat(*n)).collect::<Vec<_>>().join(" "));
    for r in rows {
        out.push('\n');
        out.push_str(r.iter().enumerate().map(|(i, v)| pad(cell(v), i)).collect::<Vec<_>>().join(" ").trim_end());
    }
    let n = rows.len();
    out.push_str(&format!("\n\n{n} row{} selected.\n", if n == 1 { "" } else { "s" }));
    out
}

/// Script Output feedback line for a statement without a result set, SQL Developer wording.
pub fn feedback(sql: &str, affected: Option<u64>) -> String {
    let words: Vec<String> = sql.split_whitespace().take(8).map(|w| w.trim_matches(['(', ';']).to_uppercase()).collect();
    let w = |i: usize| words.get(i).map(String::as_str).unwrap_or("");
    let rows = |verb: &str| {
        let n = affected.unwrap_or(0);
        format!("{n} row{} {verb}.", if n == 1 { "" } else { "s" })
    };
    let title = |s: &str| {
        let mut c = s.to_lowercase();
        if let Some(f) = c.get_mut(0..1) {
            f.make_ascii_uppercase();
        }
        c
    };
    // CREATE [OR REPLACE] [EDITIONABLE] [UNIQUE|PUBLIC|GLOBAL TEMPORARY] <TYPE> <name>
    let object = |start: usize| {
        let mut i = start;
        while matches!(w(i), "OR" | "REPLACE" | "EDITIONABLE" | "NONEDITIONABLE" | "UNIQUE" | "PUBLIC" | "GLOBAL" | "TEMPORARY" | "MATERIALIZED" | "FORCE" | "NOFORCE" | "IF" | "NOT" | "EXISTS") {
            i += 1;
        }
        let mut t = w(i).to_string();
        if t == "PACKAGE" && w(i + 1) == "BODY" {
            t = "PACKAGE BODY".into();
            i += 1;
        }
        let name = sql.split_whitespace().nth(i + 1).unwrap_or("").trim_end_matches([';', '(']).to_string();
        (title(&t), name.to_uppercase())
    };
    match w(0) {
        "INSERT" => rows("inserted"),
        "UPDATE" => rows("updated"),
        "DELETE" => rows("deleted"),
        "MERGE" => rows("merged"),
        "BEGIN" | "DECLARE" | "CALL" => "PL/SQL procedure successfully completed.".into(),
        "COMMIT" => "Commit complete.".into(),
        "ROLLBACK" => "Rollback complete.".into(),
        "SAVEPOINT" => "Savepoint created.".into(),
        "GRANT" => "Grant succeeded.".into(),
        "REVOKE" => "Revoke succeeded.".into(),
        "TRUNCATE" => format!("{} {} truncated.", title(w(1)), sql.split_whitespace().nth(2).unwrap_or("").trim_end_matches(';').to_uppercase()),
        "CREATE" | "ALTER" | "DROP" => {
            let (t, name) = object(1);
            let verb = match w(0) {
                "CREATE" if matches!(t.as_str(), "Procedure" | "Function" | "Package" | "Package body" | "Trigger" | "Type") => "compiled",
                "CREATE" => "created",
                "ALTER" => "altered",
                _ => "dropped",
            };
            format!("{t} {name} {verb}.")
        }
        "SET" | "USE" => format!("{} succeeded.", title(w(0))),
        _ => match affected {
            Some(n) if n > 0 => rows("affected"),
            _ => "Statement executed.".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(s: &str, db: Db) -> Vec<(Kind, String)> {
        split(s, db).into_iter().map(|i| (i.kind, i.text)).collect()
    }

    #[test]
    fn oracle_script() {
        let s = "select 'a;b' from dual;\n-- c;\nset serveroutput on\nbegin\n  dbms_output.put_line('x;');\nend;\n/\ncreate or replace procedure p as begin null; end;\n/\nselect q'[it's;]' from dual\n\nselect 2 from dual";
        let t = texts(s, Db::Oracle);
        assert_eq!(t[0], (Kind::Sql, "select 'a;b' from dual".into()));
        assert_eq!(t[1], (Kind::Command, "set serveroutput on".into()));
        assert_eq!(t[2], (Kind::Block, "begin\n  dbms_output.put_line('x;');\nend;".into()));
        assert_eq!(t[3].0, Kind::Block);
        assert_eq!(t[4], (Kind::Sql, "select q'[it's;]' from dual".into()));
        assert_eq!(t[5], (Kind::Sql, "select 2 from dual".into()));
        assert_eq!(t.len(), 6);
    }

    #[test]
    fn set_transaction_is_sql() {
        assert_eq!(texts("set transaction read only;", Db::Oracle), vec![(Kind::Sql, "set transaction read only".into())]);
    }

    #[test]
    fn postgres_dollar_quotes() {
        let t = texts("create function f() returns int as $$ select 1; $$ language sql;\nselect 1;", Db::Postgres);
        assert_eq!(t.len(), 2);
        assert!(t[0].1.ends_with("language sql"));
    }

    #[test]
    fn mssql_go_and_mysql_delimiter() {
        let t = texts("create procedure p as\nbegin\n select 1;\n select 2;\nend\nGO\nselect 3;", Db::Mssql);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].0, Kind::Block);
        let t = texts("DELIMITER //\ncreate procedure p() begin select 1; end//\nDELIMITER ;\nselect 2;", Db::Mysql);
        assert_eq!(t, vec![(Kind::Sql, "create procedure p() begin select 1; end".into()), (Kind::Sql, "select 2".into())]);
    }

    #[test]
    fn cursor_pick() {
        let s = "select 1 from dual;\n\nselect 2 from dual;";
        let items = split(s, Db::Oracle);
        assert_eq!(statement_at(&items, s.len()).unwrap().text, "select 2 from dual");
        assert_eq!(statement_at(&items, 3).unwrap().text, "select 1 from dual");
    }

    #[test]
    fn substitution_and_commands() {
        let mut d = Defines::default();
        assert_eq!(command("define t = EMP", &mut d), Action::Nothing);
        let mut asked = vec![];
        let r = d.substitute("select * from &t. where id = &&id and x = &id", &mut |n| {
            asked.push(n.to_string());
            Some("7".into())
        });
        assert_eq!(r.unwrap(), "select * from EMP where id = 7 and x = 7");
        assert_eq!(asked, vec!["id"]);
        assert_eq!(command("exec p(1)", &mut d), Action::Run("BEGIN p(1); END;".into()));
        assert_eq!(command("set serveroutput on", &mut d), Action::Output(true));
        assert_eq!(command("@/tmp/x.sql", &mut d), Action::Include("/tmp/x.sql".into()));
        command("set define off", &mut d);
        assert_eq!(d.substitute("a & b &x", &mut |_| None).unwrap(), "a & b &x");
    }

    #[test]
    fn feedback_and_table() {
        assert_eq!(feedback("insert into t values (1)", Some(1)), "1 row inserted.");
        assert_eq!(feedback("create or replace procedure p as begin null; end;", None), "Procedure P compiled.");
        assert_eq!(feedback("CREATE TABLE emp (id int)", None), "Table EMP created.");
        assert_eq!(feedback("drop table emp", None), "Table EMP dropped.");
        let cols = vec![crate::Column { name: "ID".into(), type_name: "NUMBER".into(), numeric: true }, crate::Column { name: "NAME".into(), type_name: "".into(), numeric: false }];
        let t = format_table(&cols, &[vec![Some("7".into()), Some("Ann".into())]]);
        assert_eq!(t, "ID NAME\n-- ----\n 7 Ann\n\n1 row selected.\n");
    }
}
