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
    ("EXIT", 4),
    ("QUIT", 4),
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

/// "PACKAGE BODY" → "Package body", as SQL*Plus feedback writes object types.
pub fn title_case(s: &str) -> String {
    let mut c = s.to_lowercase();
    if let Some(f) = c.get_mut(0..1) {
        f.make_ascii_uppercase();
    }
    c
}

/// Statement under the cursor (Ctrl/Cmd+Enter); falls back to the closest one before it.
pub fn statement_at(items: &[Item], pos: usize) -> Option<&Item> {
    items.iter().find(|it| it.start <= pos && pos <= it.end).or_else(|| items.iter().rev().find(|it| it.end <= pos)).or(items.first())
}

/// Substitution variables (`&name`, `&&name`), SQL*Plus style, and the script's SET options.
#[derive(Debug, Default, Clone)]
pub struct Defines {
    pub vars: BTreeMap<String, String>,
    pub disabled: bool,
    pub set: Settings,
}

/// SQL*Plus SET / WHENEVER options the script runner honours; layout options (PAGESIZE,
/// LINESIZE, ...) are accepted and ignored.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub feedback: bool,
    pub timing: bool,
    pub echo: bool,
    pub heading: bool,
    /// SET NULL text for NULL cells.
    pub null: String,
    /// WHENEVER SQLERROR EXIT: stop at the first error, then commit (Some(true)), roll back
    /// (Some(false)) or leave the transaction (None). Outer None = continue.
    pub on_error: Option<Option<bool>>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { feedback: true, timing: false, echo: false, heading: true, null: String::new(), on_error: None }
    }
}

/// COMMIT / ROLLBACK named among EXIT / WHENEVER words.
fn tx_word(words: &[&str]) -> Option<bool> {
    if words.contains(&"COMMIT") {
        Some(true)
    } else if words.contains(&"ROLLBACK") {
        Some(false)
    } else {
        None
    }
}

/// SET options accepted without effect (output layout of SQL*Plus).
const SET_IGNORED: &[&str] = &[
    "PAGESIZE", "LINESIZE", "VERIFY", "TERMOUT", "TRIMSPOOL", "TRIMOUT", "SQLBLANKLINES", "LONG", "LONGCHUNKSIZE", "TAB", "WRAP", "NEWPAGE", "ARRAYSIZE",
    "COLSEP", "UNDERLINE", "NUMWIDTH", "NUMFORMAT", "EMBEDDED", "SHOWMODE", "FLUSH", "AUTOPRINT", "AUTOTRACE", "SCAN", "ESCAPE", "CONCAT", "APPINFO",
    "SQLPROMPT", "SQLTERMINATOR", "TIME", "COPYCOMMIT", "EXITCOMMIT", "LOBPREFETCH", "STATEMENTCACHE", "MARKUP", "SUFFIX", "ENCODING", "SECUREDCOL",
];

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

/// Replace bind variables (`:name`, `:1`) outside strings and comments with literals: a number
/// stays as typed, empty becomes NULL, anything else is quoted. `ask` gets each distinct name
/// once (None = cancel run). Skips `::` casts, `:=`, `a[1:n]` slices and `CREATE ...` bodies
/// (trigger `:new` / `:old`).
// ponytail: literals, not real binds; Backend::execute has no bind list for queries.
pub fn bind_literals(text: &str, ask: &mut dyn FnMut(&str) -> Option<String>) -> Option<String> {
    let s = text;
    if !s.contains(':') || s[skip_trivia(s, 0, Db::Oracle)..].get(..6).is_some_and(|w| w.eq_ignore_ascii_case("CREATE")) {
        return Some(text.to_string());
    }
    let b = s.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c == b'#';
    let mut vals: BTreeMap<String, String> = BTreeMap::new();
    let mut out = String::with_capacity(s.len());
    let (mut i, mut copied) = (0, 0);
    while i < b.len() {
        let rest = &s[i..];
        if rest.starts_with("--") {
            i = next_line(s, i);
        } else if rest.starts_with("/*") {
            i = s[i + 2..].find("*/").map_or(s.len(), |n| i + 2 + n + 2);
        } else if b[i] == b'\'' || b[i] == b'"' {
            let q = b[i];
            i += 1;
            while i < b.len() && !(b[i] == q && b.get(i + 1) != Some(&q)) {
                i += if b[i] == q { 2 } else { 1 };
            }
            i += 1;
        } else if b[i] == b':' && i + 1 < b.len() && word(b[i + 1]) && (i == 0 || !(word(b[i - 1]) || b[i - 1] == b':' || b[i - 1] == b']')) {
            let end = s[i + 1..].find(|c: char| !(c.is_ascii_alphanumeric() || "_$#".contains(c))).map_or(s.len(), |n| i + 1 + n);
            let name = &s[i + 1..end];
            let key = name.to_uppercase();
            let v = match vals.get(&key) {
                Some(v) => v.clone(),
                None => {
                    let v = ask(name)?;
                    vals.insert(key, v.clone());
                    v
                }
            };
            let t = v.trim();
            let lit = if t.is_empty() {
                "NULL".to_string()
            } else if t.parse::<f64>().is_ok() && t.bytes().all(|c| c.is_ascii_digit() || b".-+eE".contains(&c)) {
                t.to_string()
            } else {
                format!("'{}'", v.replace('\'', "''"))
            };
            out.push_str(&s[copied..i]);
            out.push_str(&lit);
            i = end;
            copied = end;
        } else {
            i += b[i].len_utf8_hint();
        }
    }
    out.push_str(&s[copied.min(s.len())..]);
    Some(out)
}

/// `DELETE` / `UPDATE` with no top-level `WHERE` (one inside a subquery, string or comment does
/// not count): returns the verb, so the worksheet can confirm before touching every row.
pub fn unbounded_dml(sql: &str) -> Option<&'static str> {
    let verb = match crate::first_keyword(sql).as_str() {
        "DELETE" => "DELETE",
        "UPDATE" => "UPDATE",
        _ => return None,
    };
    let (s, b) = (sql, sql.as_bytes());
    let (mut i, mut depth) = (0, 0i32);
    while i < b.len() {
        let rest = &s[i..];
        match b[i] {
            _ if rest.starts_with("--") => i = next_line(s, i),
            _ if rest.starts_with("/*") => i = s[i + 2..].find("*/").map_or(s.len(), |n| i + 2 + n + 2),
            q @ (b'\'' | b'"' | b'`') => {
                i += 1;
                while i < b.len() && !(b[i] == q && b.get(i + 1) != Some(&q)) {
                    i += if b[i] == q { 2 } else { 1 };
                }
                i += 1;
            }
            b'(' => (depth, i) = (depth + 1, i + 1),
            b')' => (depth, i) = (depth - 1, i + 1),
            c if c.is_ascii_alphabetic() => {
                let end = rest.find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_')).map_or(s.len(), |n| i + n);
                if depth == 0 && s[i..end].eq_ignore_ascii_case("WHERE") {
                    return None;
                }
                i = end;
            }
            c => i += c.len_utf8_hint(),
        }
    }
    Some(verb)
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
    /// EXIT / QUIT: stop the script, then commit (Some(true)) / roll back (Some(false)).
    Exit(Option<bool>),
    Autocommit(bool),
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
        "SET" => {
            let val = up.split_whitespace().nth(1).unwrap_or("");
            let is = |name: &str, min: usize| arg1.len() >= min && name.starts_with(arg1);
            let on = val == "ON";
            let s = &mut defs.set;
            match () {
                _ if is("SERVEROUTPUT", 9) => return Action::Output(on),
                _ if is("DEFINE", 3) => defs.disabled = val == "OFF",
                _ if is("FEEDBACK", 4) => s.feedback = !matches!(val, "OFF" | "0"),
                _ if is("TIMING", 4) => s.timing = on,
                _ if is("ECHO", 4) => s.echo = on,
                _ if is("HEADING", 3) => s.heading = val != "OFF",
                // SET NULL keeps the text's case: "SET NULL (null)".
                _ if is("NULL", 4) => s.null = rest.split_once(char::is_whitespace).map_or("", |x| x.1).trim().trim_matches(['"', '\'']).to_string(),
                // SET AUTOCOMMIT n commits every n statements in SQL*Plus; here n > 0 = ON.
                _ if is("AUTOCOMMIT", 4) => return Action::Autocommit(on || val == "IMMEDIATE" || val.parse::<u32>().is_ok_and(|n| n > 0)),
                _ if SET_IGNORED.iter().any(|n| is(n, 3.min(n.len()))) => {}
                _ => return Action::Unsupported(t.into()),
            }
            Action::Nothing
        }
        "WHENEVER" => {
            let w: Vec<&str> = up.split_whitespace().collect();
            match (w.first().copied(), w.get(1).copied()) {
                (Some("SQLERROR"), Some("EXIT")) => defs.set.on_error = Some(tx_word(&w)),
                (Some("SQLERROR"), Some("CONTINUE")) => defs.set.on_error = None,
                (Some("OSERROR"), _) => {}
                _ => return Action::Unsupported(t.into()),
            }
            Action::Nothing
        }
        // A worksheet stays connected: EXIT only stops the script (and commits / rolls back
        // when told to; SQL*Plus would commit by default, a worksheet leaves that to the user).
        "EXIT" | "QUIT" => Action::Exit(tx_word(&up.split_whitespace().collect::<Vec<_>>())),
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


/// Script Output: the worksheet line (1-based) an output line points at. "Error starting at
/// line : N" gives N; a compiler message "L/C ..." below it gives N + L - 1.
pub fn error_target(output: &str, at: usize) -> Option<usize> {
    let lines: Vec<&str> = output.lines().collect();
    let start = |l: &str| l.trim().strip_prefix("Error starting at line : ")?.split_whitespace().next()?.parse::<usize>().ok();
    let line = lines.get(at)?;
    if let Some(n) = start(line) {
        return Some(n);
    }
    let (l, rest) = line.trim_start().split_once('/')?;
    let l: usize = l.parse().ok()?;
    rest.split(|c: char| !c.is_ascii_digit()).next()?.parse::<usize>().ok()?;
    let n = lines[..at].iter().rev().find_map(|x| start(x))?;
    Some(n + l - 1)
}

/// SQL*Plus style text table for the Script Output pane.
pub fn format_table(cols: &[crate::Column], rows: &[Vec<crate::Cell>], set: &Settings) -> String {
    if cols.is_empty() {
        return String::new();
    }
    let cell = |v: &crate::Cell| crate::lob::display(v.as_deref().unwrap_or(&set.null)).replace(['\n', '\r'], " ");
    let w: Vec<usize> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| rows.iter().map(|r| cell(&r[i]).chars().count()).chain(set.heading.then(|| c.name.chars().count())).max().unwrap_or(0).min(60))
        .collect();
    let pad = |s: String, i: usize| {
        let n = s.chars().count();
        let s: String = if n > w[i] { s.chars().take(w[i]).collect() } else { s };
        let fill = " ".repeat(w[i].saturating_sub(s.chars().count()));
        if cols[i].numeric { format!("{fill}{s}") } else { format!("{s}{fill}") }
    };
    let mut lines = vec![];
    if set.heading {
        lines.push(cols.iter().enumerate().map(|(i, c)| pad(c.name.clone(), i)).collect::<Vec<_>>().join(" ").trim_end().to_string());
        lines.push(w.iter().map(|n| "-".repeat(*n)).collect::<Vec<_>>().join(" "));
    }
    for r in rows {
        lines.push(r.iter().enumerate().map(|(i, v)| pad(cell(v), i)).collect::<Vec<_>>().join(" ").trim_end().to_string());
    }
    let mut out = lines.join("\n");
    let n = rows.len();
    if set.feedback {
        out.push_str(&format!("\n\n{n} row{} selected.", if n == 1 { "" } else { "s" }));
    }
    out.push('\n');
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
    let title = title_case;
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
    fn unbounded() {
        assert_eq!(unbounded_dml("delete from emp"), Some("DELETE"));
        assert_eq!(unbounded_dml("UPDATE emp SET sal = (SELECT max(sal) FROM emp WHERE 1 = 1) -- where"), Some("UPDATE"));
        assert_eq!(unbounded_dml("update emp set note = 'where' "), Some("UPDATE"));
        assert_eq!(unbounded_dml("delete from emp where id = 1"), None);
        assert_eq!(unbounded_dml("UPDATE emp SET x = 1\nWHERE id IN (SELECT id FROM t)"), None);
        assert_eq!(unbounded_dml("select * from emp"), None);
    }

    #[test]
    fn binds() {
        let mut asked = vec![];
        let r = bind_literals("select ':x' from t -- :c\nwhere id = :id and n = :Name and id2 = :ID and t = x::int and a := 1 and s[1:n]", &mut |n| {
            asked.push(n.to_string());
            Some(if n == "id" { "7".into() } else { "O'Neil".into() })
        });
        assert_eq!(r.unwrap(), "select ':x' from t -- :c\nwhere id = 7 and n = 'O''Neil' and id2 = 7 and t = x::int and a := 1 and s[1:n]");
        assert_eq!(asked, vec!["id", "Name"]);
        assert_eq!(bind_literals("x = :1", &mut |_| Some(" ".into())).unwrap(), "x = NULL");
        let trig = "create trigger t before insert on e for each row begin :new.id := 1; end;";
        assert_eq!(bind_literals(trig, &mut |_| None).unwrap(), trig);
        assert!(bind_literals("x = :a", &mut |_| None).is_none());
    }

    #[test]
    fn feedback_and_table() {
        assert_eq!(feedback("insert into t values (1)", Some(1)), "1 row inserted.");
        assert_eq!(feedback("create or replace procedure p as begin null; end;", None), "Procedure P compiled.");
        assert_eq!(feedback("CREATE TABLE emp (id int)", None), "Table EMP created.");
        assert_eq!(feedback("drop table emp", None), "Table EMP dropped.");
        let cols = vec![crate::Column { name: "ID".into(), type_name: "NUMBER".into(), numeric: true }, crate::Column { name: "NAME".into(), type_name: "".into(), numeric: false }];
        let t = format_table(&cols, &[vec![Some("7".into()), Some("Ann".into())]], &Settings::default());
        assert_eq!(t, "ID NAME\n-- ----\n 7 Ann\n\n1 row selected.\n");
        let quiet = Settings { heading: false, feedback: false, null: "-".into(), ..Settings::default() };
        assert_eq!(format_table(&cols, &[vec![Some("7".into()), None]], &quiet), "7 -\n");
    }

    #[test]
    fn error_lines() {
        let out = "Table T created.\n\nError starting at line : 7 in command -\ncreate procedure p as\nError report -\nProcedure P compiled with errors\nLINE/COL ERROR\n3/3 PLS-00201: identifier 'X' must be declared\n";
        assert_eq!(error_target(out, 2), Some(7));
        assert_eq!(error_target(out, 7), Some(9));
        assert_eq!(error_target(out, 6), None);
        assert_eq!(error_target(out, 0), None);
        assert_eq!(error_target("12/4 x", 0), None);
    }

    #[test]
    fn set_whenever_exit() {
        let mut d = Defines::default();
        for c in ["SET FEEDBACK OFF", "set timing on", "SET ECHO ON", "SET HEA OFF", "SET NULL '(null)'", "SET PAGESIZE 0", "SET LINES 200", "SET VERIFY OFF"] {
            assert_eq!(command(c, &mut d), Action::Nothing, "{c}");
        }
        assert_eq!(d.set, Settings { feedback: false, timing: true, echo: true, heading: false, null: "(null)".into(), on_error: None });
        assert_eq!(command("SET AUTOCOMMIT ON", &mut d), Action::Autocommit(true));
        assert_eq!(command("SET AUTOCOMMIT OFF", &mut d), Action::Autocommit(false));
        assert_eq!(command("SET SERVEROUTPUT ON SIZE UNLIMITED", &mut d), Action::Output(true));
        assert!(matches!(command("SET FOO BAR", &mut d), Action::Unsupported(_)));
        command("WHENEVER SQLERROR EXIT SQL.SQLCODE ROLLBACK", &mut d);
        assert_eq!(d.set.on_error, Some(Some(false)));
        command("WHENEVER SQLERROR EXIT", &mut d);
        assert_eq!(d.set.on_error, Some(None));
        command("WHENEVER SQLERROR CONTINUE", &mut d);
        assert_eq!(d.set.on_error, None);
        assert_eq!(command("exit", &mut d), Action::Exit(None));
        assert_eq!(command("QUIT COMMIT", &mut d), Action::Exit(Some(true)));
        assert_eq!(command_name("exit;"), Some("EXIT"));
        assert_eq!(command_name("existing"), None);
    }
}
