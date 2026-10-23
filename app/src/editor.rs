//! SQL code editor: StyledTextCtrl with a small container-lexer highlighter
//! (wxDragon's STC does not expose SetKeyWords, so keywords are styled here).

use wxdragon::prelude::*;

const S_DEFAULT: i32 = 0;
const S_COMMENT: i32 = 1;
const S_STRING: i32 = 2;
const S_NUMBER: i32 = 3;
const S_KEYWORD: i32 = 4;
const S_IDENT_Q: i32 = 5;
pub const MARK_ERROR: i32 = 1;
pub const MARK_BOOKMARK: i32 = 2;
// Scintilla STYLE_BRACELIGHT / STYLE_BRACEBAD.
const S_BRACE: i32 = 34;
const S_BRACE_BAD: i32 = 35;

pub const KEYWORDS: &[&str] = &[
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
    // Lines with PL/SQL compilation errors: tinted background.
    stc.marker_define_symbol(MARK_ERROR, MarkerSymbol::Background, rgb(p.deleted_bg), rgb(p.deleted_bg));
    stc.marker_define_symbol(MARK_BOOKMARK, MarkerSymbol::RoundRect, rgb(p.accent), rgb(p.accent));
    stc.style_set_background(33, rgb(margin)); // STYLE_LINENUMBER
    stc.style_set_foreground(33, rgb(p.ed_margin_text));
    stc.style_set_foreground(S_BRACE, rgb(p.accent));
    stc.style_set_bold(S_BRACE, true);
    stc.style_set_foreground(S_BRACE_BAD, rgb(0xcc3333));
    stc.style_set_bold(S_BRACE_BAD, true);
    // SCLEX_CONTAINER = 0 (wxDragon's Lexer enum is off: its Container = 2 is SCLEX_PYTHON).
    stc.set_lexer(0);
    stc.set_margin_type_typed(0, MarginType::Number);
    stc.set_margin_width(0, 36);
    // Bookmark symbols.
    stc.set_margin_width(1, 12);
    stc.on_stc_update_ui(move |_| brace_light(&stc));
    crate::copy::editor(stc);
    stc.set_tab_width(4);
    stc.set_use_tabs(false);
    stc.set_caret_line_visible(true);
    stc.set_caret_line_background(rgb(line));
    stc.on_stc_style_needed(move |_| highlight(&stc));
    stc
}

/// Bracket at or just before the caret, if any.
fn brace_at(stc: &StyledTextCtrl) -> Option<i32> {
    let p = stc.get_current_pos();
    let is = |q: i32| q >= 0 && matches!(u8::try_from(stc.get_char_at(q)), Ok(b'(' | b')' | b'[' | b']'));
    [p, p - 1].into_iter().find(|q| is(*q))
}

fn brace_light(stc: &StyledTextCtrl) {
    match brace_at(stc) {
        Some(a) => match stc.brace_match(a) {
            -1 => stc.brace_bad_light(a),
            b => stc.brace_highlight(a, b),
        },
        None => stc.brace_highlight(-1, -1),
    }
}

/// Edit ▸ Go to Matching Bracket.
pub fn goto_brace(stc: StyledTextCtrl) {
    if let Some(b) = brace_at(&stc).map(|a| stc.brace_match(a)).filter(|b| *b >= 0) {
        stc.goto_pos(b);
    }
}

/// Byte range of the whole lines touched by the selection (or the caret line).
fn line_block(stc: &StyledTextCtrl) -> (i32, i32) {
    let (s, e) = (stc.get_selection_start(), stc.get_selection_end());
    let l0 = stc.line_from_position(s);
    let mut l1 = stc.line_from_position(e);
    if e > s && e == stc.position_from_line(l1) {
        l1 -= 1;
    }
    let end = if l1 + 1 < stc.get_line_count() { stc.position_from_line(l1 + 1) } else { stc.get_length() };
    (stc.position_from_line(l0), end)
}

fn replace(stc: &StyledTextCtrl, a: i32, b: i32, text: &str) {
    stc.set_target_start(a);
    stc.set_target_end(b);
    stc.replace_target(text);
}

/// Edit ▸ Duplicate Lines: the touched lines, copied below themselves (one undo step).
pub fn duplicate_lines(stc: StyledTextCtrl) {
    let (a, b) = line_block(&stc);
    let text = stc.get_text();
    let Some(block) = text.get(a as usize..b as usize) else { return };
    let block = if block.ends_with('\n') { block.to_string() } else { format!("{block}\n") };
    replace(&stc, a, b, &format!("{block}{}", &text[a as usize..b as usize]));
    stc.set_selection(a + block.len() as i32, a + 2 * block.len() as i32 - 1);
}

/// Edit ▸ Move Lines Up / Down: swap the touched lines with the line above / below.
pub fn move_lines(stc: StyledTextCtrl, up: bool) {
    let (a, b) = line_block(&stc);
    let text = stc.get_text();
    let (lo, hi) = if up {
        if a == 0 {
            return;
        }
        (stc.position_from_line(stc.line_from_position(a) - 1), b)
    } else {
        if b >= stc.get_length() {
            return;
        }
        let nl = stc.line_from_position(b);
        (a, if nl + 1 < stc.get_line_count() { stc.position_from_line(nl + 1) } else { stc.get_length() })
    };
    let (Some(moving), Some(other), Some(span)) = (text.get(a as usize..b as usize), if up { text.get(lo as usize..a as usize) } else { text.get(b as usize..hi as usize) }, text.get(lo as usize..hi as usize)) else { return };
    // Swap the lines without their line ends; the span keeps its own trailing newline (or none
    // on the last line).
    let eol = if span.contains("\r\n") { "\r\n" } else { "\n" };
    let (m, o) = (moving.trim_end_matches(['\r', '\n']), other.trim_end_matches(['\r', '\n']));
    let tail = if span.ends_with('\n') { eol } else { "" };
    let out = if up { format!("{m}{eol}{o}{tail}") } else { format!("{o}{eol}{m}{tail}") };
    replace(&stc, lo, hi, &out);
    let start = if up { lo } else { lo + (o.len() + eol.len()) as i32 };
    stc.set_selection(start, start + m.len() as i32);
}

/// Edit ▸ Toggle Bookmark on the caret line.
pub fn toggle_bookmark(stc: StyledTextCtrl) {
    let line = stc.line_from_position(stc.get_current_pos());
    if stc.marker_get(line) & (1 << MARK_BOOKMARK) != 0 {
        stc.marker_delete(line, MARK_BOOKMARK);
    } else {
        stc.marker_add(line, MARK_BOOKMARK);
    }
}

/// Edit ▸ Next / Previous Bookmark, wrapping around.
pub fn goto_bookmark(stc: StyledTextCtrl, next: bool) {
    let (line, mask, last) = (stc.line_from_position(stc.get_current_pos()), 1 << MARK_BOOKMARK, stc.get_line_count() - 1);
    let found = if next {
        Some(stc.marker_next(line + 1, mask)).filter(|l| *l >= 0).unwrap_or_else(|| stc.marker_next(0, mask))
    } else {
        Some(stc.marker_previous(line - 1, mask)).filter(|l| *l >= 0).unwrap_or_else(|| stc.marker_previous(last, mask))
    };
    if found >= 0 {
        stc.goto_line(found);
    }
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
