//! SQL code editor: StyledTextCtrl with a small container-lexer highlighter
//! (wxDragon's STC does not expose SetKeyWords, so keywords are styled here).

use wxdragon::prelude::*;

const S_DEFAULT: i32 = 0;
const S_COMMENT: i32 = 1;
const S_STRING: i32 = 2;
const S_NUMBER: i32 = 3;
const S_KEYWORD: i32 = 4;
const S_IDENT_Q: i32 = 5;

const KEYWORDS: &[&str] = &[
    "ADD", "ALL", "ALTER", "AND", "ANY", "AS", "ASC", "BEGIN", "BETWEEN", "BODY", "BY", "CASE", "CHECK", "CLOSE", "COLUMN", "COMMENT",
    "COMMIT", "CONSTRAINT", "CREATE", "CROSS", "CURSOR", "DECLARE", "DEFAULT", "DELETE", "DESC", "DISTINCT", "DROP", "ELSE", "ELSIF",
    "END", "EXCEPTION", "EXECUTE", "EXISTS", "EXIT", "FETCH", "FIRST", "FOR", "FOREIGN", "FROM", "FULL", "FUNCTION", "GRANT", "GROUP",
    "HAVING", "IF", "IN", "INDEX", "INNER", "INSERT", "INTERSECT", "INTO", "IS", "JOIN", "KEY", "LAST", "LEFT", "LIKE", "LIMIT", "LOOP",
    "MERGE", "MINUS", "NOT", "NULL", "NULLS", "OF", "OFFSET", "ON", "OPEN", "OR", "ORDER", "OUTER", "OVER", "PACKAGE", "PARTITION",
    "PRIMARY", "PROCEDURE", "RAISE", "REFERENCES", "REPLACE", "RETURN", "RETURNING", "REVOKE", "RIGHT", "ROLLBACK", "ROWS", "SAVEPOINT",
    "SELECT", "SEQUENCE", "SET", "SYNONYM", "TABLE", "THEN", "TO", "TOP", "TRIGGER", "TRUNCATE", "TYPE", "UNION", "UNIQUE", "UPDATE",
    "USING", "VALUES", "VIEW", "WHEN", "WHERE", "WHILE", "WITH",
];

pub fn new_editor(parent: &dyn WxWidget, font_face: &str, font_size: i32) -> StyledTextCtrl {
    let stc = StyledTextCtrl::builder(parent).build();
    let p = crate::theme::p();
    let rgb = crate::theme::c;
    let (bg, fg, kw, st, num, cm, line, margin) = (p.ed_bg, p.ed_text, p.ed_keyword, p.ed_string, p.ed_number, p.ed_comment, p.ed_caret_line, p.ed_margin);
    if let Some(f) = Font::new_with_details(font_size, FontFamily::Teletype as i32, 0, 400, false, font_face) {
        stc.style_set_font(S_DEFAULT, &f);
        stc.style_set_font(32, &f); // STYLE_DEFAULT
    }
    stc.style_set_background(32, rgb(bg));
    stc.style_set_foreground(32, rgb(fg));
    stc.style_clear_all();
    stc.style_set_foreground(S_COMMENT, rgb(cm));
    stc.style_set_foreground(S_STRING, rgb(st));
    stc.style_set_foreground(S_NUMBER, rgb(num));
    stc.style_set_foreground(S_KEYWORD, rgb(kw));
    stc.style_set_bold(S_KEYWORD, true);
    stc.style_set_foreground(S_IDENT_Q, rgb(fg));
    stc.style_set_background(33, rgb(margin)); // STYLE_LINENUMBER
    stc.style_set_foreground(33, rgb(p.ed_margin_text));
    // SCLEX_CONTAINER = 0 (wxDragon's Lexer enum is off: its Container = 2 is SCLEX_PYTHON).
    stc.set_lexer(0);
    stc.set_margin_type_typed(0, MarginType::Number);
    stc.set_margin_width(0, 36);
    stc.set_margin_width(1, 4);
    stc.set_tab_width(4);
    stc.set_use_tabs(false);
    stc.set_caret_line_visible(true);
    stc.set_caret_line_background(rgb(line));
    stc.on_stc_style_needed(move |_| highlight(&stc));
    stc
}

/// Style the whole document. ponytail: full restyle per change, fine below ~1 MB scripts.
pub fn highlight(stc: &StyledTextCtrl) {
    let text = stc.get_text();
    let b = text.as_bytes();
    stc.start_styling(0);
    let mut i = 0;
    let run = |len: usize, style: i32| stc.set_styling(len as i32, style);
    while i < b.len() {
        let start = i;
        let c = b[i];
        let style = if c == b'-' && b.get(i + 1) == Some(&b'-') {
            i = text[i..].find('\n').map_or(b.len(), |n| i + n);
            S_COMMENT
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i = text[i + 2..].find("*/").map_or(b.len(), |n| i + 2 + n + 2);
            S_COMMENT
        } else if c == b'\'' || c == b'"' || c == b'`' {
            i += 1;
            while i < b.len() {
                if b[i] == c {
                    if b.get(i + 1) == Some(&c) {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            if c == b'\'' { S_STRING } else { S_IDENT_Q }
        } else if c.is_ascii_digit() {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.') {
                i += 1;
            }
            S_NUMBER
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'$' || b[i] == b'#') {
                i += 1;
            }
            let w = text[start..i].to_ascii_uppercase();
            if KEYWORDS.binary_search(&w.as_str()).is_ok() { S_KEYWORD } else { S_DEFAULT }
        } else {
            // Non-ASCII and punctuation: advance one UTF-8 sequence.
            i += match c {
                0xf0.. => 4,
                0xe0.. => 3,
                0xc0.. => 2,
                _ => 1,
            };
            S_DEFAULT
        };
        run(i.min(b.len()) - start, style);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn keywords_sorted() {
        assert!(super::KEYWORDS.windows(2).all(|w| w[0] < w[1]));
    }
}
