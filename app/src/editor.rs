//! SQL code editor: StyledTextCtrl with a small container-lexer highlighter
//! (wxDragon's STC does not expose SetKeyWords, so keywords are styled here).

use wxdragon::prelude::*;

const S_DEFAULT: i32 = 0;
const S_COMMENT: i32 = 1;
const S_STRING: i32 = 2;
const S_NUMBER: i32 = 3;
const S_KEYWORD: i32 = 4;
const S_IDENT_Q: i32 = 5;
/// Object names while Ctrl / Cmd is held: blue, underlined, clickable (Ctrl+Click opens them).
const S_LINK: i32 = 6;
/// Alias colours 8..15 (an alias, its uses and what it stands for share one); 16..23 are the
/// same colours underlined, for aliases while Ctrl / Cmd is held.
const S_ALIAS0: i32 = 8;
const ALIAS_COLOURS: i32 = 8;
/// ponytail: aliases are found only in statements up to 64 KB (one rescan per keystroke stays cheap).
const ALIAS_SPAN: usize = 64 * 1024;
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
    stc.style_set_foreground(S_LINK, rgb(p.link));
    stc.style_set_underline(S_LINK, true);
    for (i, c) in p.ed_alias.iter().enumerate() {
        let i = i as i32;
        stc.style_set_foreground(S_ALIAS0 + i, rgb(*c));
        stc.style_set_foreground(S_ALIAS0 + ALIAS_COLOURS + i, rgb(*c));
        stc.style_set_underline(S_ALIAS0 + ALIAS_COLOURS + i, true);
    }
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
    // Incremental styling. wxDragon has no GetEndStyled, so the first unstyled byte is tracked
    // here: text changes (Modified) move it back, styling moves it forward. Changes made by
    // the styling itself are ignored (`busy`).
    let styled = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let busy = std::rc::Rc::new(std::cell::Cell::new(false));
    // Ctrl / Cmd held: object names show as links (whole text restyled once per press).
    let links = std::rc::Rc::new(std::cell::Cell::new(false));
    let toggle = {
        let (styled, busy, links) = (styled.clone(), busy.clone(), links.clone());
        move |on: bool| {
            if links.replace(on) != on {
                styled.set(0);
                highlight(&stc, &styled, &busy, on, None);
            }
        }
    };
    let t1 = toggle.clone();
    stc.on_key_down(move |e| {
        if let WindowEventData::Keyboard(k) = &e {
            if k.get_key_code() == Some(wxdragon::keycode::WXK_COMMAND) {
                t1(true);
            }
        }
        e.skip(true);
    });
    let t2 = toggle.clone();
    stc.on_key_up(move |e| {
        if let WindowEventData::Keyboard(k) = &e {
            if k.get_key_code() == Some(wxdragon::keycode::WXK_COMMAND) {
                t2(false);
            }
        }
        e.skip(true);
    });
    // A key-up missed while another window had focus must not leave the links on.
    stc.on_kill_focus(move |e| {
        toggle(false);
        e.skip(true);
    });
    let (s1, b1) = (styled.clone(), busy.clone());
    stc.on_stc_modified(move |e| {
        if !b1.get() {
            if let Some(p) = e.get_position() {
                s1.set(s1.get().min(p.max(0) as usize));
            }
        }
    });
    stc.on_stc_style_needed(move |e| highlight(&stc, &styled, &busy, links.get(), e.get_position()));
    stc
}

/// Read-only highlighted SQL (DDL panes, script previews, confirmations), editor font.
pub fn sql_view(parent: &dyn WxWidget, text: &str) -> StyledTextCtrl {
    let prefs = crate::app().prefs();
    let stc = new_editor(parent, &prefs.editor_font, prefs.editor_font_size);
    show(&stc, text);
    stc
}

/// Replace the text of a read-only [`sql_view`].
pub fn show(stc: &StyledTextCtrl, text: &str) {
    stc.set_read_only(false);
    stc.set_text(text);
    stc.set_read_only(true);
}

/// Append to a read-only [`sql_view`] and scroll to the end.
pub fn append(stc: &StyledTextCtrl, text: &str) {
    stc.set_read_only(false);
    stc.append_text(text);
    stc.set_read_only(true);
    stc.goto_pos(stc.get_length());
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
/// Style from the first unstyled line up to `until` (Scintilla asks for what it shows), not
/// the whole document on every keystroke.
fn highlight(stc: &StyledTextCtrl, styled: &std::cell::Cell<usize>, busy: &std::cell::Cell<bool>, links: bool, until: Option<i32>) {
    let text = stc.get_text();
    let len = text.len();
    let until = until.map_or(len, |p| (p.max(0) as usize).min(len));
    // Start of the line holding the first unstyled byte (a token boundary)...
    let mut from = styled.get().min(len);
    from = text[..from].rfind('\n').map_or(0, |i| i + 1);
    // ...moved up while that line starts inside a multi-line comment or string.
    while from > 0 && matches!(stc.get_style_at(from as i32 - 1), S_COMMENT | S_STRING | S_IDENT_Q) {
        from = text[..from - 1].rfind('\n').map_or(0, |i| i + 1);
    }
    // Links need the statement's start (FROM may be lines above): restyle from the top then.
    // Aliases need their whole statement (the alias is defined after its first uses).
    let mut until = until;
    while !text.is_char_boundary(until) {
        until += 1;
    }
    let mut aliases = true;
    if links {
        from = 0;
    } else {
        match statement_start(stc, &text, from) {
            Some(s) => from = s,
            None => aliases = false,
        }
    }
    let mut cap = (until + ALIAS_SPAN).min(len);
    while !text.is_char_boundary(cap) {
        cap -= 1;
    }
    match text[until..cap].find(';') {
        Some(k) => until += k + 1,
        None if cap == len => until = len,
        None => aliases = false,
    }
    let mut toks = lex_links(&text, from, until, links);
    if aliases {
        alias_styles(&text, from, &mut toks, links);
    }
    busy.set(true);
    stc.start_styling(from as i32);
    let mut end = from;
    for (n, style) in toks {
        stc.set_styling(n as i32, style);
        end += n;
    }
    busy.set(false);
    styled.set(end);
}

/// Start of the statement holding byte `pos`: just after the last real `;` (not in a string or
/// comment) before it, or 0. None when that is more than [`ALIAS_SPAN`] back.
pub fn statement_start(stc: &StyledTextCtrl, text: &str, pos: usize) -> Option<usize> {
    let mut floor = pos.saturating_sub(ALIAS_SPAN);
    while !text.is_char_boundary(floor) {
        floor += 1;
    }
    let mut at = pos;
    loop {
        match text[floor..at].rfind(';') {
            Some(k) if stc.get_style_at((floor + k) as i32) == S_DEFAULT => return Some(floor + k + 1),
            Some(k) => at = floor + k,
            None => return (floor == 0).then_some(0),
        }
    }
}

/// [`lex`], with object names marked [`S_LINK`] when `links` (Ctrl / Cmd held): the names after
/// FROM, JOIN, INTO, UPDATE and TABLE, each part of `owner.name`, and every item of a FROM list.
fn lex_links(text: &str, from: usize, until: usize, links: bool) -> Vec<(usize, i32)> {
    let mut toks = lex(text, from, until);
    if !links {
        return toks;
    }
    let (mut expect, mut in_from, mut dot_next, mut after_dot) = (false, false, false, false);
    let mut at = from;
    for (n, style) in toks.iter_mut() {
        let t = &text[at..at + *n];
        at += *n;
        if t.trim().is_empty() {
            continue;
        }
        let ident = *style == S_IDENT_Q || (*style == S_DEFAULT && t.starts_with(|c: char| c.is_alphabetic() || c == '_'));
        if *style == S_KEYWORD {
            match t.to_ascii_uppercase().as_str() {
                "FROM" | "JOIN" => (expect, in_from) = (true, true),
                "INTO" | "UPDATE" | "TABLE" => (expect, in_from) = (true, false),
                "AS" => {}
                _ => (expect, in_from) = (false, false),
            }
            (dot_next, after_dot) = (false, false);
        } else if ident {
            if expect || after_dot {
                *style = S_LINK;
                (expect, after_dot, dot_next) = (false, false, true);
            } else {
                dot_next = false;
            }
        } else if t == "." && dot_next {
            *style = S_LINK;
            (after_dot, dot_next) = (true, false);
        } else {
            if t == "," && in_from {
                expect = true;
            } else if t == "(" {
                expect = false;
            }
            (dot_next, after_dot) = (false, false);
        }
    }
    toks
}

/// One alias of a statement: its definition and the tokens it stands for (indices into the
/// statement's significant tokens).
struct Alias {
    name: String,
    def: usize,
    target: (usize, usize),
    /// Subquery / CTE: only its brackets take the colour.
    parens: bool,
    /// CTE name: bare uses (`from q`) count, not only `q.col`.
    cte: bool,
}

/// Words the lexer leaves unstyled that are still not aliases.
const NOT_ALIAS: &[&str] = &["APPLY", "CONNECT", "FORCE", "IGNORE", "LATERAL", "MODEL", "NATURAL", "PIVOT", "SAMPLE", "START", "STRAIGHT_JOIN", "UNPIVOT", "USE", "WINDOW"];

/// Significant token: (text, style, index into the token list, byte offset).
type Sig<'a> = (&'a str, i32, usize, usize);

/// Non-blank, non-comment tokens of `toks` (lexed from byte `from`).
fn significant<'a>(text: &'a str, from: usize, toks: &[(usize, i32)]) -> Vec<Sig<'a>> {
    let mut at = from;
    let mut out = vec![];
    for (i, (n, style)) in toks.iter().enumerate() {
        let t = &text[at..at + n];
        if *style != S_COMMENT && !t.trim().is_empty() {
            out.push((t, *style, i, at));
        }
        at += n;
    }
    out
}

fn is_ident(t: &Sig) -> bool {
    t.1 == S_IDENT_Q || t.1 == S_LINK || (t.1 == S_DEFAULT && t.0.starts_with(|c: char| c.is_alphabetic() || c == '_'))
}

/// Name as the database compares it: quoted kept, unquoted upper-cased.
fn alias_name(t: &str) -> String {
    match t.as_bytes().first() {
        Some(b'"' | b'`') => t[1..t.len().saturating_sub(1).max(1)].to_string(),
        _ => t.to_ascii_uppercase(),
    }
}

/// Aliases of one statement: `[owner.]table [AS] a` and `(subquery) [AS] a` after FROM / JOIN /
/// UPDATE / INTO / USING and commas of a FROM list, and `WITH name AS (...)`.
fn find_aliases(sig: &[Sig]) -> Vec<Alias> {
    let n = sig.len();
    let ident = |i: usize| i < n && is_ident(&sig[i]);
    let kw = |i: usize| (i < n && sig[i].1 == S_KEYWORD).then(|| sig[i].0.to_ascii_uppercase());
    let is = |i: usize, p: &str| i < n && sig[i].0 == p;
    let mut close = vec![usize::MAX; n];
    let mut open = vec![];
    for i in 0..n {
        if is(i, "(") {
            open.push(i);
        } else if is(i, ")") {
            if let Some(o) = open.pop() {
                close[o] = i;
            }
        }
    }
    // `[AS] name` right after an item ending at `end`.
    let alias_after = |end: usize| {
        let k = if kw(end + 1).as_deref() == Some("AS") { end + 2 } else { end + 1 };
        (ident(k) && !is(k + 1, ".") && !is(k + 1, "(") && NOT_ALIAS.binary_search(&sig[k].0.to_ascii_uppercase().as_str()).is_err()).then_some(k)
    };
    let mut out = vec![];
    let (mut expect, mut in_from, mut cte) = (false, false, false);
    // Per open bracket: the outer `in_from`, and the bracket itself when it opens a FROM subquery.
    let mut stack: Vec<(bool, Option<usize>)> = vec![];
    let mut cte_ends = vec![];
    let mut i = 0;
    while i < n {
        if let Some(k) = kw(i) {
            match k.as_str() {
                "FROM" | "JOIN" => (expect, in_from) = (true, true),
                "UPDATE" | "INTO" | "USING" => (expect, in_from) = (true, false),
                "WITH" => (expect, in_from, cte) = (false, false, true),
                _ => (expect, in_from) = (false, false),
            }
        } else if cte && ident(i) {
            cte = false;
            let mut j = i + 1;
            if is(j, "(") && close[j] != usize::MAX {
                j = close[j] + 1;
            }
            if kw(j).as_deref() == Some("AS") {
                j += 1;
                // [NOT] MATERIALIZED
                while j < n && !is(j, "(") && (ident(j) || kw(j).as_deref() == Some("NOT")) {
                    j += 1;
                }
                if is(j, "(") && close[j] != usize::MAX {
                    out.push(Alias { name: alias_name(sig[i].0), def: i, target: (j, close[j]), parens: true, cte: true });
                    cte_ends.push(close[j]);
                }
            }
        } else if expect && ident(i) {
            let start = i;
            while is(i + 1, ".") && ident(i + 2) {
                i += 2;
            }
            expect = false;
            // A table function `f(...)` is skipped.
            if let Some(k) = alias_after(i).filter(|_| !is(i + 1, "(")) {
                out.push(Alias { name: alias_name(sig[k].0), def: k, target: (start, i), parens: false, cte: false });
                i = k;
            }
        } else if is(i, "(") {
            let sub = expect && matches!(kw(i + 1).as_deref(), Some("SELECT" | "WITH")) && close[i] != usize::MAX;
            stack.push((in_from, sub.then_some(i)));
            (expect, in_from) = (false, false);
        } else if is(i, ")") {
            let (f, sub) = stack.pop().unwrap_or((false, None));
            in_from = f;
            if let Some(k) = sub.and_then(|_| alias_after(i)) {
                out.push(Alias { name: alias_name(sig[k].0), def: k, target: (sub.unwrap(), i), parens: true, cte: false });
                i = k + 1;
                continue;
            }
            cte = cte_ends.contains(&i) && is(i + 1, ",");
        } else if is(i, ",") {
            expect = in_from;
        } else {
            expect = false;
        }
        i += 1;
    }
    out
}

/// The alias that token `i` defines or uses (`a.col`, or a bare CTE name).
fn alias_of(sig: &[Sig], aliases: &[Alias], i: usize) -> Option<usize> {
    if !is_ident(&sig[i]) || (i > 0 && sig[i - 1].0 == ".") {
        return None;
    }
    let name = alias_name(sig[i].0);
    let dotted = sig.get(i + 1).is_some_and(|t| t.0 == ".");
    aliases.iter().position(|a| a.def == i || (a.name == name && (dotted || a.cte)))
}

/// Colour every alias of each statement in `toks` (lexed from byte `from`): its definition, its
/// uses and what it stands for share one of [`ALIAS_COLOURS`]. With `links`, aliases are
/// underlined and table names keep their link style.
fn alias_styles(text: &str, from: usize, toks: &mut [(usize, i32)], links: bool) {
    let sig = significant(text, from, toks);
    for stmt in sig.split(|t| t.0 == ";" && t.1 == S_DEFAULT) {
        let aliases = find_aliases(stmt);
        let colour = |n: usize| S_ALIAS0 + n as i32 % ALIAS_COLOURS;
        for (n, a) in aliases.iter().enumerate() {
            let (t0, t1) = a.target;
            for t in if a.parens { vec![t0, t1] } else { (t0..=t1).collect() } {
                let tok = &mut toks[stmt[t].2];
                if tok.1 != S_LINK {
                    tok.1 = colour(n);
                }
            }
        }
        for i in 0..stmt.len() {
            if let Some(n) = alias_of(stmt, &aliases, i) {
                toks[stmt[i].2].1 = colour(n) + if links { ALIAS_COLOURS } else { 0 };
            }
        }
    }
}

/// Ctrl+Click on an alias at byte `pos`: the byte range of what it stands for (the table name,
/// or the subquery / CTE with its brackets).
pub fn alias_target_at(text: &str, pos: usize) -> Option<(usize, usize)> {
    let toks = lex(text, 0, text.len());
    let sig = significant(text, 0, &toks);
    let stmt = sig.split(|t| t.0 == ";" && t.1 == S_DEFAULT).find(|s| s.last().is_some_and(|t| t.3 + t.0.len() >= pos))?;
    let i = stmt.iter().position(|t| t.3 <= pos && pos <= t.3 + t.0.len() && is_ident(t))?;
    let aliases = find_aliases(stmt);
    let a = &aliases[alias_of(stmt, &aliases, i)?];
    let (t0, t1) = (&stmt[a.target.0], &stmt[a.target.1]);
    Some((t0.3, t1.3 + t1.0.len()))
}

/// Tokens (byte length, style) from byte `from` (a token boundary) through the token that
/// reaches `until`.
fn lex(text: &str, from: usize, until: usize) -> Vec<(usize, i32)> {
    let b = text.as_bytes();
    let mut out = vec![];
    let mut i = from;
    let mut run = |len: usize, style: i32| out.push((len, style));
    while i < b.len() && i < until.max(from + 1) {
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
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQL: &str = "select a, 'x''y' from t -- note\n/* multi\nline */ where n = 42;";

    #[test]
    fn lex_whole_text() {
        let toks = lex(SQL, 0, SQL.len());
        assert_eq!(toks.iter().map(|t| t.0).sum::<usize>(), SQL.len());
        assert_eq!(toks[0], (6, S_KEYWORD));
        assert!(toks.contains(&(6, S_STRING)) && toks.contains(&(16, S_COMMENT)) && toks.contains(&(2, S_NUMBER)));
    }

    #[test]
    fn lex_from_a_token_boundary_matches_the_tail() {
        let full = lex(SQL, 0, SQL.len());
        // Start at "from" (offset 17): same tokens as the full run from there on.
        let tail = lex(SQL, 17, SQL.len());
        let mut at = 0;
        let skip = full.iter().take_while(|t| {
            at += t.0;
            at <= 17
        });
        assert_eq!(tail, full[skip.count()..].to_vec());
    }

    #[test]
    fn lex_stops_after_the_token_reaching_until() {
        let toks = lex(SQL, 0, 3);
        assert_eq!(toks, vec![(6, S_KEYWORD)]);
    }

    /// Text of each token styled `style`.
    fn styled(sql: &str, toks: &[(usize, i32)], style: i32) -> Vec<String> {
        let mut at = 0;
        let mut out = vec![];
        for (n, s) in toks {
            if *s == style {
                out.push(sql[at..at + n].to_string());
            }
            at += n;
        }
        out
    }

    #[test]
    fn links_mark_object_names_after_from_join_into() {
        let sql = "select x from hr.emp e, dept d join loc l on l.id = e.id where e.x = 1;\ninsert into audit_log values (1)";
        let toks = lex_links(sql, 0, sql.len(), true);
        assert_eq!(styled(sql, &toks, S_LINK), vec!["hr", ".", "emp", "dept", "loc", "audit_log"]);
        // Without Ctrl the same text has no links.
        assert!(styled(sql, &lex_links(sql, 0, sql.len(), false), S_LINK).is_empty());
    }

    /// Texts styled with alias colour `i`.
    fn alias_texts(sql: &str, i: i32) -> Vec<String> {
        let mut toks = lex(sql, 0, sql.len());
        alias_styles(sql, 0, &mut toks, false);
        styled(sql, &toks, S_ALIAS0 + i)
    }

    #[test]
    fn table_aliases_share_a_colour_with_their_table() {
        let sql = "select e.x, d.y from hr.emp e join dept as d on d.id = e.d";
        assert_eq!(alias_texts(sql, 0), vec!["e", "hr", ".", "emp", "e", "e"]);
        assert_eq!(alias_texts(sql, 1), vec!["d", "dept", "d", "d"]);
    }

    #[test]
    fn subquery_and_cte_aliases_colour_their_brackets() {
        let sql = "with q as (select 1 a from dual) select q.a, s.n from q, (select 2 n from dual) s";
        assert_eq!(alias_texts(sql, 0), vec!["q", "(", ")", "q", "q"]);
        assert_eq!(alias_texts(sql, 1), vec!["s", "(", ")", "s"]);
    }

    #[test]
    fn ctrl_click_on_an_alias_selects_what_it_stands_for() {
        let sql = "select s.n, e.x from (select 2 n from dual) s, hr.emp e;\nselect 1 from dual";
        let at = |needle: &str| sql.find(needle).unwrap();
        let sub = "(select 2 n from dual)";
        assert_eq!(alias_target_at(sql, at("s.n")), Some((at(sub), at(sub) + sub.len())));
        assert_eq!(alias_target_at(sql, at("e.x")), Some((at("hr.emp"), at("hr.emp") + 6)));
        // Not an alias: the table name itself, or text in another statement.
        assert_eq!(alias_target_at(sql, at("emp e")), None);
        assert_eq!(alias_target_at(sql, sql.len() - 2), None);
    }

    #[test]
    fn keywords_sorted() {
        assert!(super::KEYWORDS.windows(2).all(|w| w[0] < w[1]));
    }
}
