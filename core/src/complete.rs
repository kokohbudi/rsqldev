//! Code completion helpers: the word being typed at the cursor, and the tables a statement
//! names (with aliases), so the editor can offer keywords, tables and columns.

use crate::registry::Kind as Db;

#[derive(Debug, PartialEq)]
pub struct Ctx {
    /// Partial word before the cursor (may be empty).
    pub prefix: String,
    /// `x` in `x.pre|`: an alias, table or schema name.
    pub qualifier: Option<String>,
}

fn ident_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '$' | '#' | '"' | '`' | '[' | ']')
}

/// Start of the identifier ending at byte `end`.
fn word_start(text: &str, end: usize) -> usize {
    text[..end].char_indices().rev().take_while(|(_, c)| ident_char(*c)).last().map_or(end, |(i, _)| i)
}

/// Word before byte offset `pos` and its `x.` qualifier.
pub fn context(text: &str, pos: usize) -> Ctx {
    let pos = pos.min(text.len());
    let start = word_start(text, pos);
    let prefix = text[start..pos].to_string();
    let qualifier = text[..start].strip_suffix('.').map(|before| before[word_start(before, before.len())..].to_string()).filter(|q| !q.is_empty());
    Ctx { prefix, qualifier }
}

#[derive(Debug, PartialEq)]
pub struct TableRef {
    pub owner: Option<String>,
    pub name: String,
    pub alias: Option<String>,
}

/// Words that end a table reference instead of being its alias.
const STOP: &[&str] = &[
    "WHERE", "JOIN", "INNER", "LEFT", "RIGHT", "FULL", "CROSS", "OUTER", "NATURAL", "ON", "USING", "GROUP", "ORDER", "HAVING", "UNION",
    "INTERSECT", "MINUS", "EXCEPT", "SET", "VALUES", "LIMIT", "OFFSET", "FETCH", "FOR", "START", "CONNECT", "WINDOW", "QUALIFY",
    "PIVOT", "UNPIVOT", "SAMPLE", "PARTITION", "SELECT", "RETURNING", "LATERAL", "APPLY", "WITH", "DEFAULT",
];

#[derive(Debug, PartialEq)]
enum Tok {
    Word(String),
    Punct(char),
}

fn tokens(sql: &str) -> Vec<Tok> {
    let b: Vec<char> = sql.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == '-' && b.get(i + 1) == Some(&'-') {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && b.get(i + 1) == Some(&'*') {
            i += 2;
            while i < b.len() && !(b[i] == '*' && b.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
        } else if c == '\'' {
            i += 1;
            while i < b.len() && b[i] != '\'' {
                i += 1;
            }
            i += 1;
        } else if matches!(c, '"' | '`' | '[') {
            let close = if c == '[' { ']' } else { c };
            let s = i;
            i += 1;
            while i < b.len() && b[i] != close {
                i += 1;
            }
            i += 1;
            out.push(Tok::Word(b[s..i.min(b.len())].iter().collect()));
        } else if c.is_alphanumeric() || matches!(c, '_' | '$' | '#') {
            let s = i;
            while i < b.len() && (b[i].is_alphanumeric() || matches!(b[i], '_' | '$' | '#')) {
                i += 1;
            }
            out.push(Tok::Word(b[s..i].iter().collect()));
        } else {
            if !c.is_whitespace() {
                out.push(Tok::Punct(c));
            }
            i += 1;
        }
    }
    out
}

/// Tables named after FROM / JOIN / UPDATE / INTO (and comma lists after FROM), with aliases.
/// ponytail: token scan, not a parser; subquery sources `(...) x` are skipped.
pub fn tables(sql: &str) -> Vec<TableRef> {
    let t = tokens(sql);
    let word = |i: usize| match t.get(i) {
        Some(Tok::Word(w)) => Some(w.as_str()),
        _ => None,
    };
    let kw = |i: usize, k: &str| word(i).is_some_and(|w| w.eq_ignore_ascii_case(k));
    let mut out = vec![];
    let mut i = 0;
    while i < t.len() {
        let starts = ["FROM", "JOIN", "UPDATE", "INTO"].iter().any(|k| kw(i, k));
        i += 1;
        if !starts {
            continue;
        }
        // One reference, then more while a comma follows (FROM a, b).
        loop {
            let Some(first) = word(i).filter(|w| !STOP.iter().any(|s| w.eq_ignore_ascii_case(s))) else { break };
            let (owner, name) = if t.get(i + 1) == Some(&Tok::Punct('.')) && word(i + 2).is_some() {
                i += 3;
                (Some(first.to_string()), word(i - 1).unwrap().to_string())
            } else {
                i += 1;
                (None, first.to_string())
            };
            if kw(i, "AS") {
                i += 1;
            }
            let alias = word(i).filter(|w| !STOP.iter().any(|s| w.eq_ignore_ascii_case(s))).map(str::to_string);
            if alias.is_some() {
                i += 1;
            }
            out.push(TableRef { owner, name, alias });
            if t.get(i) == Some(&Tok::Punct(',')) {
                i += 1;
            } else {
                break;
            }
        }
    }
    out
}

/// Cursor sits where a table name goes: right after FROM / JOIN / INTO / UPDATE / TABLE /
/// DESC[RIBE], or after a comma in a FROM list. Drives the automatic table popup.
pub fn expects_table(text: &str, pos: usize) -> bool {
    let pos = pos.min(text.len());
    let before = &text[..word_start(text, pos)];
    // A qualifier (`x.`) is column / schema completion, not this.
    if before.ends_with('.') {
        return false;
    }
    let t = tokens(before);
    let is_kw = |tok: &Tok, ks: &[&str]| matches!(tok, Tok::Word(w) if ks.iter().any(|k| w.eq_ignore_ascii_case(k)));
    match t.last() {
        Some(tok) if is_kw(tok, &["FROM", "JOIN", "INTO", "UPDATE", "TABLE", "DESC", "DESCRIBE"]) => true,
        Some(Tok::Punct(',')) => {
            // Comma inside a FROM list: the nearest clause keyword before it is FROM.
            let mut depth = 0i32;
            for tok in t.iter().rev() {
                match tok {
                    Tok::Punct(')') => depth += 1,
                    Tok::Punct('(') if depth == 0 => return false,
                    Tok::Punct('(') => depth -= 1,
                    _ if depth > 0 => {}
                    tok if is_kw(tok, &["FROM"]) => return true,
                    tok if is_kw(tok, &["SELECT", "WHERE", "SET", "ON", "BY", "VALUES", "HAVING", "INTO", "USING"]) => return false,
                    _ => {}
                }
            }
            false
        }
        _ => false,
    }
}

/// Caret inside a VALUES tuple of an INSERT: the target table, the columns the statement lists
/// (None = all, in table order) and which value is being typed (0-based).
#[derive(Debug, PartialEq)]
pub struct InsertHint {
    pub owner: Option<String>,
    pub table: String,
    pub columns: Option<Vec<String>>,
    pub arg: usize,
}

/// [`InsertHint`] for the caret at byte `pos` of statement `sql`. Only the text before the
/// caret is read: tuples on later lines / rows (`values (..), (..)`) and Oracle
/// `INSERT ALL INTO .. VALUES ..` (the nearest INTO) work.
pub fn insert_hint(sql: &str, pos: usize) -> Option<InsertHint> {
    let t = tokens(&sql[..pos.min(sql.len())]);
    let word = |i: usize| match t.get(i) {
        Some(Tok::Word(w)) => Some(w.as_str()),
        _ => None,
    };
    let kw = |i: usize, k: &str| word(i).is_some_and(|w| w.eq_ignore_ascii_case(k));
    let punct = |i: usize, c: char| t.get(i) == Some(&Tok::Punct(c));
    // Open brackets with their top-level comma count, and each closed bracket's opener.
    let mut open: Vec<(usize, usize)> = vec![];
    let mut opener = vec![usize::MAX; t.len()];
    for (i, tok) in t.iter().enumerate() {
        match tok {
            Tok::Punct('(') => open.push((i, 0)),
            Tok::Punct(')') => {
                if let Some((o, _)) = open.pop() {
                    opener[i] = o;
                }
            }
            Tok::Punct(',') => {
                if let Some(last) = open.last_mut() {
                    last.1 += 1;
                }
            }
            _ => {}
        }
    }
    // The VALUES keyword of a tuple opened at `o`: right before it, or before `(..), (..),`.
    let values_of = |mut o: usize| loop {
        if o > 0 && kw(o - 1, "VALUES") {
            return Some(o - 1);
        }
        if o < 2 || !punct(o - 1, ',') || !punct(o - 2, ')') || opener[o - 2] == usize::MAX {
            return None;
        }
        o = opener[o - 2];
    };
    // Innermost open bracket that is a tuple (the caret may be inside a function call in it).
    let (v, arg) = open.iter().rev().find_map(|&(o, commas)| values_of(o).map(|v| (v, commas)))?;
    let mut i = v.checked_sub(1)?;
    let columns = if punct(i, ')') && opener[i] != usize::MAX {
        let o = opener[i];
        let cols: Vec<String> = t[o + 1..i].split(|x| *x == Tok::Punct(',')).filter_map(|c| match c.last() {
            Some(Tok::Word(w)) => Some(w.clone()),
            _ => None,
        }).collect();
        i = o.checked_sub(1)?;
        Some(cols)
    } else {
        None
    };
    let name = word(i)?.to_string();
    let owner = if i >= 2 && punct(i - 1, '.') { word(i - 2).map(str::to_string) } else { None };
    let into = i.checked_sub(if owner.is_some() { 3 } else { 1 })?;
    kw(into, "INTO").then_some(InsertHint { owner, table: name, columns, arg })
}

/// The table a qualifier (`e` in `e.`) points at: alias first, then table name.
pub fn resolve<'a>(refs: &'a [TableRef], q: &str) -> Option<&'a TableRef> {
    let q = unquote(q);
    refs.iter()
        .find(|r| r.alias.as_deref().is_some_and(|a| unquote(a).eq_ignore_ascii_case(&q)))
        .or_else(|| refs.iter().find(|r| unquote(&r.name).eq_ignore_ascii_case(&q)))
}

fn unquote(s: &str) -> String {
    s.trim_matches(['"', '`', '[', ']']).to_string()
}

/// Identifier as the dictionary stores it: quoted names verbatim, others folded.
pub fn stored(db: Db, raw: &str) -> String {
    if raw.starts_with(['"', '`', '[']) { unquote(raw) } else { crate::dialect::fold(db, raw) }
}

/// Text to insert for a candidate: in the case the user is typing, quoted when needed. Keywords always follow the prefix; names
/// only on engines that fold unquoted identifiers (Oracle, PostgreSQL) and only when the stored
/// name is the folded form, so the inserted text still means the same object.
pub fn match_case(db: Db, prefix: &str, word: &str, keyword: bool) -> String {
    let lower = prefix.chars().any(char::is_lowercase) || (prefix.is_empty() && !keyword && db == Db::Oracle);
    let plain = word.chars().all(|c| c.is_alphanumeric() || c == '_') && crate::dialect::fold(db, word) == word;
    if !keyword && !plain && (matches!(db, Db::Oracle | Db::Postgres) || !word.chars().all(|c| c.is_alphanumeric() || c == '_')) {
        return crate::dialect::quote(db, word); // mixed case / special characters need quotes
    }
    if !(keyword || (matches!(db, Db::Oracle | Db::Postgres) && plain)) {
        return word.to_string();
    }
    if lower {
        word.to_lowercase()
    } else if prefix.is_empty() && !keyword {
        word.to_string()
    } else {
        word.to_uppercase()
    }
}

/// Candidates starting with `prefix` (case-insensitive), sorted, without duplicates.
pub fn filter(prefix: &str, words: impl IntoIterator<Item = String>) -> Vec<String> {
    let p = prefix.to_lowercase();
    let mut v: Vec<String> = words.into_iter().filter(|w| w.to_lowercase().starts_with(&p)).collect();
    v.sort_by_key(|w| w.to_lowercase());
    v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_context() {
        assert_eq!(context("select e.na", 11), Ctx { prefix: "na".into(), qualifier: Some("e".into()) });
        assert_eq!(context("select e.", 9), Ctx { prefix: "".into(), qualifier: Some("e".into()) });
        assert_eq!(context("select * from em", 16), Ctx { prefix: "em".into(), qualifier: None });
        assert_eq!(context("select \"Emp\".", 13).qualifier.as_deref(), Some("\"Emp\""));
    }

    #[test]
    fn table_refs() {
        let r = tables("select * from hr.emp e, jobs join dept d on d.id = e.dept_id where x in (select 1 from dual)");
        assert_eq!(r[0], TableRef { owner: Some("hr".into()), name: "emp".into(), alias: Some("e".into()) });
        assert_eq!(r[1], TableRef { owner: None, name: "jobs".into(), alias: None });
        assert_eq!(r[2], TableRef { owner: None, name: "dept".into(), alias: Some("d".into()) });
        assert_eq!(r[3].name, "dual");
        let r = tables("update \"Emp\" as x set a = 1 where -- from fake\n b = 'from y'");
        assert_eq!(r, vec![TableRef { owner: None, name: "\"Emp\"".into(), alias: Some("x".into()) }]);
        assert_eq!(resolve(&r, "X").unwrap().name, "\"Emp\"");
        assert_eq!(stored(Db::Oracle, "emp"), "EMP");
        assert_eq!(stored(Db::Oracle, "\"Emp\""), "Emp");
    }

    #[test]
    fn table_position() {
        let at = |s: &str| expects_table(s, s.len());
        assert!(at("select * from "));
        assert!(at("select * from em"));
        assert!(at("select * FROM emp e join de"));
        assert!(at("select * from emp e, de"));
        assert!(at("insert into "));
        assert!(at("update em"));
        assert!(!at("select a, b"), "comma in select list");
        assert!(!at("select * from emp where a in (1, "));
        assert!(!at("select * from emp e where e."));
        assert!(!at("select na"));
        assert!(!at("select * from emp -- from\nwhere x"));
    }

    #[test]
    fn typed_case() {
        assert_eq!(match_case(Db::Oracle, "em", "EMP", false), "emp");
        assert_eq!(match_case(Db::Oracle, "Em", "EMP", false), "emp");
        assert_eq!(match_case(Db::Oracle, "EM", "EMP", false), "EMP");
        assert_eq!(match_case(Db::Oracle, "my", "MyTable", false), "\"MyTable\"", "mixed-case name needs quotes");
        assert_eq!(match_case(Db::Mysql, "my", "my table", false), "`my table`");
        assert_eq!(match_case(Db::Postgres, "EM", "emp", false), "EMP");
        assert_eq!(match_case(Db::Mysql, "em", "Emp", false), "Emp");
        assert_eq!(match_case(Db::Mysql, "sel", "SELECT", true), "select");
        assert_eq!(match_case(Db::Mysql, "", "SELECT", true), "SELECT");
    }

    #[test]
    fn filtering() {
        assert_eq!(filter("em", ["EMP".into(), "dept".into(), "Employees".into(), "emp".into()]), vec!["EMP", "Employees"]);
    }

    fn hint(sql: &str) -> Option<InsertHint> {
        insert_hint(&sql.replace('|', ""), sql.find('|').unwrap())
    }

    #[test]
    fn insert_hint_with_listed_columns_counts_top_level_commas() {
        let h = hint("insert into hr.emp (id, name, hired)\nvalues (1, upper('a,b'), to_date('x', |").unwrap();
        assert_eq!((h.owner.as_deref(), h.table.as_str(), h.arg), (Some("hr"), "emp", 2));
        assert_eq!(h.columns, Some(vec!["id".to_string(), "name".into(), "hired".into()]));
    }

    #[test]
    fn insert_hint_without_columns_and_on_later_rows() {
        let h = hint("insert into emp values\n  (1, 'a'),\n  (2, |").unwrap();
        assert_eq!((h.owner, h.table.as_str(), h.columns, h.arg), (None, "emp", None, 1));
        // Oracle INSERT ALL: the INTO nearest the tuple.
        let h = hint("insert all into a (x) values (1) into b (y, z) values (2, |").unwrap();
        assert_eq!((h.table.as_str(), h.arg), ("b", 1));
    }

    #[test]
    fn no_insert_hint_outside_a_values_tuple() {
        assert_eq!(hint("insert into emp (id, |"), None);
        assert_eq!(hint("insert into emp values (1) |"), None);
        assert_eq!(hint("select f(1, |"), None);
        assert_eq!(hint("insert into emp select max(|"), None);
        assert_eq!(hint("a.b values (|"), None);
    }
}
