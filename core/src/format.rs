//! SQL formatter (Ctrl+Shift+F / Ctrl+F7): uppercase keywords, one clause per line, select-list and
//! AND / OR items on their own indented lines, subqueries indented. Only whitespace and keyword
//! case change: strings, quoted identifiers and comments are kept verbatim. PL/SQL blocks and
//! SQL*Plus commands are left as they are.

use crate::registry::Kind as Db;
use crate::script::{self, Kind};

const KEYWORDS: &[&str] = &[
    "ADD", "ALL", "ALTER", "AND", "ANY", "AS", "ASC", "BETWEEN", "BY", "CASE", "CREATE", "CROSS", "DEFAULT", "DELETE", "DESC", "DISTINCT",
    "DROP", "ELSE", "END", "EXCEPT", "EXISTS", "FETCH", "FIRST", "FOR", "FROM", "FULL", "GROUP", "HAVING", "IN", "INNER", "INSERT",
    "INTERSECT", "INTO", "IS", "JOIN", "LAST", "LEFT", "LIKE", "LIMIT", "MERGE", "MINUS", "NEXT", "NOT", "NULL", "NULLS", "OFFSET", "ON",
    "ONLY", "OR", "ORDER", "OUTER", "OVER", "PARTITION", "RETURNING", "RIGHT", "ROW", "ROWS", "SELECT", "SET", "TABLE", "THEN", "TOP",
    "UNION", "UPDATE", "USING", "VALUES", "VIEW", "WHEN", "WHERE", "WITH",
];

/// Clauses that start a new line at the statement's indent.
const CLAUSES: &[&str] = &[
    "SELECT", "FROM", "WHERE", "GROUP", "ORDER", "HAVING", "UNION", "INTERSECT", "MINUS", "EXCEPT", "LIMIT", "OFFSET", "FETCH", "SET",
    "VALUES", "RETURNING", "INSERT", "UPDATE", "DELETE", "MERGE", "WITH", "USING",
];
/// Join keywords: a new line unless they continue a join keyword (LEFT OUTER JOIN).
const JOINS: &[&str] = &["JOIN", "INNER", "LEFT", "RIGHT", "FULL", "CROSS", "NATURAL"];

#[derive(Debug, PartialEq, Clone)]
enum Tok {
    Word(String),
    /// String literal, quoted identifier or block comment: kept verbatim.
    Raw(String),
    LineComment(String),
    Punct(char),
}

fn tokens(sql: &str) -> Vec<Tok> {
    let b: Vec<char> = sql.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    let until = |i: &mut usize, end: &dyn Fn(usize) -> bool| {
        while *i < b.len() && !end(*i) {
            *i += 1;
        }
    };
    while i < b.len() {
        let c = b[i];
        let s = i;
        if c.is_whitespace() {
            i += 1;
        } else if c == '-' && b.get(i + 1) == Some(&'-') {
            until(&mut i, &|j| b[j] == '\n');
            out.push(Tok::LineComment(b[s..i].iter().collect()));
        } else if c == '/' && b.get(i + 1) == Some(&'*') {
            i += 2;
            until(&mut i, &|j| b[j] == '*' && b.get(j + 1) == Some(&'/'));
            i = (i + 2).min(b.len());
            out.push(Tok::Raw(b[s..i].iter().collect()));
        } else if matches!(c, '\'' | '"' | '`' | '[') {
            let close = if c == '[' { ']' } else { c };
            i += 1;
            loop {
                until(&mut i, &|j| b[j] == close);
                i += 1;
                // Doubled quote = escaped quote.
                if b.get(i) == Some(&close) && close != ']' {
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(Tok::Raw(b[s..i.min(b.len())].iter().collect()));
        } else if c.is_alphanumeric() || matches!(c, '_' | '$' | '#' | '@') || (c == ':' && b.get(i + 1).is_some_and(|n| n.is_alphanumeric()) && (i == 0 || b[i - 1] != ':')) {
            i += 1;
            until(&mut i, &|j| !(b[j].is_alphanumeric() || matches!(b[j], '_' | '$' | '#' | '@' | '.')));
            out.push(Tok::Word(b[s..i].iter().collect()));
        } else {
            i += 1;
            out.push(Tok::Punct(c));
        }
    }
    out
}

fn is(t: &Tok, k: &str) -> bool {
    matches!(t, Tok::Word(w) if w.eq_ignore_ascii_case(k))
}

fn any(t: &Tok, ks: &[&str]) -> bool {
    ks.iter().any(|k| is(t, k))
}

/// Format one SQL statement.
pub fn statement(sql: &str) -> String {
    let t = tokens(sql);
    let mut out = String::new();
    // Paren stack: true = subquery (indented block), false = inline group (function args, IN lists).
    let mut parens: Vec<bool> = vec![];
    let mut level = 0usize;
    let pad = |n: usize| "    ".repeat(n);
    let newline = |out: &mut String, n: usize| {
        let trimmed = out.trim_end_matches(' ').len();
        out.truncate(trimmed);
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&pad(n));
    };
    // Clause whose items are listed one per line (SELECT a, b / SET x = 1, y = 2 / GROUP BY).
    let mut list_clause = false;
    let mut case_depth = 0;
    let mut between = false;
    for (i, tok) in t.iter().enumerate() {
        let prev = i.checked_sub(1).map(|p| &t[p]);
        let inline = parens.last() == Some(&false);
        match tok {
            Tok::Word(w) => {
                let up = w.to_ascii_uppercase();
                let word = if KEYWORDS.binary_search(&up.as_str()).is_ok() { up } else { w.clone() };
                let prev_is = |ks: &[&str]| prev.is_some_and(|p| any(p, ks));
                if is(tok, "CASE") {
                    case_depth += 1;
                } else if is(tok, "END") && case_depth > 0 {
                    case_depth -= 1;
                }
                let top = !inline && case_depth == 0;
                let clause = (any(tok, CLAUSES) && !(is(tok, "SET") && prev_is(&["UPDATE"])) && !(is(tok, "ALL") || prev_is(&["INSERT", "DELETE"])))
                    || (any(tok, JOINS) && !prev_is(&["INNER", "LEFT", "RIGHT", "FULL", "CROSS", "NATURAL", "OUTER"]));
                if top && clause {
                    newline(&mut out, level);
                    list_clause = any(tok, &["SELECT", "SET", "GROUP", "ORDER"]);
                } else if top && any(tok, &["AND", "OR"]) && !(is(tok, "AND") && between) {
                    newline(&mut out, level + 1);
                } else if !out.is_empty() && !out.ends_with(['\n', ' ', '(']) && !out.ends_with("::") && !matches!(prev, Some(Tok::Punct('.'))) {
                    out.push(' ');
                }
                if is(tok, "BETWEEN") {
                    between = true;
                } else if is(tok, "AND") {
                    between = false;
                }
                out.push_str(&word);
                // List clauses: items start on the next line, indented.
                let opens_list = (is(tok, "SELECT") && !t.get(i + 1).is_some_and(|n| any(n, &["DISTINCT", "ALL", "TOP"])))
                    || (is(tok, "DISTINCT") && prev_is(&["SELECT"]))
                    || (is(tok, "BY") && prev_is(&["GROUP", "ORDER"]))
                    || (is(tok, "SET") && !prev_is(&["UPDATE"]));
                if top && list_clause && opens_list {
                    newline(&mut out, level + 1);
                }
            }
            Tok::Raw(r) => {
                if !out.is_empty() && !out.ends_with(['\n', ' ', '(']) && !matches!(prev, Some(Tok::Punct('.'))) {
                    out.push(' ');
                }
                out.push_str(r);
            }
            Tok::LineComment(c) => {
                if !out.is_empty() && !out.ends_with(['\n', ' ']) {
                    out.push(' ');
                }
                out.push_str(c);
                newline(&mut out, level + usize::from(list_clause));
            }
            Tok::Punct('(') => {
                let sub = t.get(i + 1).is_some_and(|n| any(n, &["SELECT", "WITH"]));
                // No space only for function calls: `count(`, but `INTO t (a, b)`, `IN (`, `OVER (`.
                let call = matches!(prev, Some(Tok::Word(_))) && !prev.is_some_and(|p| any(p, KEYWORDS)) && !i.checked_sub(2).is_some_and(|pp| is(&t[pp], "INTO"));
                if !call && !out.is_empty() && !out.ends_with(['\n', ' ', '(']) {
                    out.push(' ');
                }
                out.push('(');
                parens.push(sub);
                if sub {
                    level += 1;
                    list_clause = false;
                }
            }
            Tok::Punct(')') => {
                if parens.pop() == Some(true) {
                    level = level.saturating_sub(1);
                    newline(&mut out, level);
                }
                let trimmed = out.trim_end_matches(' ').len();
                out.truncate(trimmed);
                out.push(')');
            }
            Tok::Punct(',') => {
                let trimmed = out.trim_end_matches(' ').len();
                out.truncate(trimmed);
                out.push(',');
                if list_clause && !inline {
                    newline(&mut out, level + 1);
                } else {
                    out.push(' ');
                }
            }
            Tok::Punct('.') => {
                let trimmed = out.trim_end_matches(' ').len();
                out.truncate(trimmed);
                out.push('.');
            }
            Tok::Punct(';') => {
                let trimmed = out.trim_end_matches(' ').len();
                out.truncate(trimmed);
                out.push(';');
            }
            // `a::type` casts stick to both sides.
            Tok::Punct(':') if matches!(prev, Some(Tok::Punct(':'))) || matches!(t.get(i + 1), Some(Tok::Punct(':'))) => {
                let trimmed = out.trim_end_matches(' ').len();
                out.truncate(trimmed);
                out.push(':');
            }
            Tok::Punct(c) => {
                // Operators: spaced; `::` casts and `||` stay together.
                let glue = matches!(prev, Some(Tok::Punct(p)) if (*p == ':' && *c == ':') || (*p == '|' && *c == '|') || matches!((p, c), ('<', '=' | '>') | ('>' | '!', '=')));
                if !glue && !out.is_empty() && !out.ends_with(['\n', ' ', '(']) {
                    out.push(' ');
                }
                out.push(*c);
                if !matches!(c, ':' | '|') || !matches!(t.get(i + 1), Some(Tok::Punct(n)) if n == c) {
                    if !matches!(t.get(i + 1), Some(Tok::Punct('=' | '>')) if matches!(c, '<' | '>' | '!')) {
                        out.push(' ');
                    }
                }
            }
        }
    }
    out.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim().to_string()
}

/// Format every SQL statement in a script, leaving blocks, commands and the text between
/// statements (terminators, blank lines) as they are.
pub fn script(text: &str, db: Db) -> String {
    let mut out = text.to_string();
    for it in script::split(text, db).iter().rev().filter(|it| it.kind == Kind::Sql) {
        let original = &text[it.start..it.end];
        let lead = original.len() - original.trim_start().len();
        let body = original.trim();
        out.replace_range(it.start + lead..it.start + lead + body.len(), &statement(body));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Formatting only changes whitespace and keyword case.
    #[test]
    fn same_tokens() {
        let norm = |s: &str| -> Vec<Tok> {
            tokens(s).into_iter().map(|t| if let Tok::Word(w) = t { Tok::Word(w.to_uppercase()) } else { t }).collect()
        };
        for s in [
            "select `a b`, \"Q\".x, [y] from t1 /* c */ where z like 'a''b%' -- tail\n and q = $1 or r <= :b and s != 3",
            "with x as (select 1 n from dual) select n, sum(n) over (partition by n order by n) from x union all select 2, 3 from dual",
            "delete from t where id in (select id from u where v between 1 and 9) returning id",
            "select a.b.c, f(x, g(y)), -1 * 2, 'x' || 'y', e'\\n' from s.t",
        ] {
            assert_eq!(norm(&statement(s)), norm(s), "{s}\n=>\n{}", statement(s));
        }
    }

    #[test]
    fn keyword_table_sorted() {
        assert!(KEYWORDS.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn select() {
        let f = statement("select a.id, b.name , count(*) cnt from emp a left outer join dept b on b.id = a.dept_id and b.x = 'it''s from' where a.sal between 1 and 2 or a.y in (1, 2) group by a.id, b.name order by 3 desc");
        assert_eq!(
            f,
            "SELECT\n    a.id,\n    b.name,\n    count(*) cnt\nFROM emp a\nLEFT OUTER JOIN dept b ON b.id = a.dept_id\n    AND b.x = 'it''s from'\nWHERE a.sal BETWEEN 1 AND 2\n    OR a.y IN (1, 2)\nGROUP BY\n    a.id,\n    b.name\nORDER BY\n    3 DESC"
        );
    }

    #[test]
    fn subquery_and_dml() {
        assert_eq!(
            statement("select * from (select x from t where y = 1) s"),
            "SELECT\n    *\nFROM (\n    SELECT\n        x\n    FROM t\n    WHERE y = 1\n) s"
        );
        assert_eq!(statement("update t set a = 1, b = 'x' where id = :id"), "UPDATE t\nSET\n    a = 1,\n    b = 'x'\nWHERE id = :id");
        assert_eq!(statement("insert into t (a, b) values (1, 'v')"), "INSERT INTO t (a, b)\nVALUES (1, 'v')");
        assert_eq!(statement("select a::text, b || c from t where a >= 1 and b <> 2"), "SELECT\n    a::text,\n    b || c\nFROM t\nWHERE a >= 1\n    AND b <> 2");
    }

    #[test]
    fn keeps_text_and_blocks() {
        let s = "select 1 from dual -- note\n;\nbegin\n  null;\nend;\n/\nset serveroutput on\n";
        let f = script(s, Db::Oracle);
        assert!(f.starts_with("SELECT\n    1\nFROM dual -- note"), "{f}");
        assert!(f.contains("begin\n  null;\nend;\n/\nset serveroutput on"), "{f}");
        assert_eq!(statement("select case when a = 1 then 'x' else 'y' end from t"), "SELECT\n    CASE WHEN a = 1 THEN 'x' ELSE 'y' END\nFROM t");
    }
}
