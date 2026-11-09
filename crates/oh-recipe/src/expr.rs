//! Tiny deterministic languages used by cards: calc expressions, rule conditions, text formats.
use crate::error::{Error, Result};
use rust_decimal::{Decimal, RoundingStrategy};
use std::str::FromStr;

pub fn parse_decimal(s: &str) -> Option<Decimal> {
    let t: String = s.chars().filter(|c| !c.is_whitespace() && *c != ',').collect();
    if t.is_empty() {
        return None;
    }
    Decimal::from_str(&t).ok()
}

/// "YYYY-MM-DD" -> (y, m, d) with calendar validation.
pub fn parse_date(s: &str) -> Option<(u32, u32, u32)> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let y: u32 = s[0..4].parse().ok()?;
    let m: u32 = s[5..7].parse().ok()?;
    let d: u32 = s[8..10].parse().ok()?;
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let dim = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if leap { 29 } else { 28 },
        _ => return None,
    };
    (1..=dim).contains(&d).then_some((y, m, d))
}

fn tokens(src: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let cs: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            i += 1;
        } else if "(),".contains(c) {
            out.push(c.to_string());
            i += 1;
        } else if c == '\'' {
            let mut j = i + 1;
            while j < cs.len() && cs[j] != '\'' {
                j += 1;
            }
            if j >= cs.len() {
                return Err(Error::new("invalid_expression", src));
            }
            out.push(cs[i..=j].iter().collect());
            i = j + 1;
        } else if c == '=' || c == '!' {
            if cs.get(i + 1) != Some(&'=') {
                return Err(Error::new("invalid_expression", src));
            }
            out.push(format!("{c}="));
            i += 2;
        } else if c.is_alphanumeric() || c == '_' || c == '.' {
            let mut j = i;
            while j < cs.len() && (cs[j].is_alphanumeric() || cs[j] == '_' || cs[j] == '.') {
                j += 1;
            }
            out.push(cs[i..j].iter().collect());
            i = j;
        } else {
            return Err(Error::new("invalid_expression", src));
        }
    }
    Ok(out)
}

// ---- calc expressions ----
#[derive(Debug, Clone)]
pub enum Expr {
    Num(Decimal),
    Str(String),
    Ref(String),
    Call(String, Vec<Expr>),
}

pub fn parse_expr(src: &str) -> Result<Expr> {
    let t = tokens(src)?;
    let mut pos = 0;
    let e = expr(&t, &mut pos, src)?;
    if pos != t.len() {
        return Err(Error::new("invalid_expression", src));
    }
    Ok(e)
}
fn expr(t: &[String], pos: &mut usize, src: &str) -> Result<Expr> {
    let bad = || Error::new("invalid_expression", src);
    let tok = t.get(*pos).ok_or_else(bad)?.clone();
    *pos += 1;
    if let Ok(d) = Decimal::from_str(&tok) {
        return Ok(Expr::Num(d));
    }
    if tok.len() >= 2 && tok.starts_with('\'') {
        return Ok(Expr::Str(tok[1..tok.len() - 1].to_string()));
    }
    if t.get(*pos).map(String::as_str) == Some("(") {
        *pos += 1;
        let mut args = Vec::new();
        if t.get(*pos).map(String::as_str) == Some(")") {
            *pos += 1;
        } else {
            loop {
                args.push(expr(t, pos, src)?);
                match t.get(*pos).map(String::as_str) {
                    Some(",") => *pos += 1,
                    Some(")") => {
                        *pos += 1;
                        break;
                    }
                    _ => return Err(bad()),
                }
            }
        }
        const FUNCS: &[&str] = &[
            "sum", "mul", "fx", "vat", "round_krw", "sub", "div", "neg", "abs", "pos", "if_pos", "coalesce", "date_add", "days", "weekday", "grade", "grade_basis", "rate",
            "discount", "reg_status", "discount_status", "segs", "nights", "per_diem", "per_diem_note", "grades", "grades_ok", "items_krw", "items_usd", "items_note",
            "itin_check", "list_len", "seg_join", "segs_check", "if_eq", "between",
            "round_to", "pct", "share", "share_rest", "prorate", "add_months", "months_between", "list_sum",
        ];
        if !FUNCS.contains(&tok.as_str()) {
            return Err(Error::new("unknown_function", tok));
        }
        return Ok(Expr::Call(tok, args));
    }
    if tok.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') {
        Ok(Expr::Ref(tok))
    } else {
        Err(bad())
    }
}
pub fn expr_refs(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Num(_) | Expr::Str(_) => {}
        Expr::Ref(r) => out.push(r.clone()),
        Expr::Call(_, a) => a.iter().for_each(|x| expr_refs(x, out)),
    }
}
pub fn round_krw(d: Decimal) -> Decimal {
    d.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
}
// ---- rule conditions ----
#[derive(Debug, Clone)]
pub enum Atom {
    Always,
    Eq(String, String, bool),
    Present(String),
    Absent(String),
    DateLe(String, String),
    AnyPresent(Vec<String>),
    /// |a - b| <= tol (numbers; `tol` and either side may be a literal). False if unresolved.
    Near(String, String, String),
    /// a <= b (numbers). False if unresolved.
    Le(String, String),
}
fn is_num_lit(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_digit()) && parse_decimal(s).is_some()
}
fn arg_num(s: &str, get: &dyn Fn(&str) -> Option<String>) -> Option<Decimal> {
    if is_num_lit(s) { parse_decimal(s) } else { get(s).and_then(|v| parse_decimal(&v)) }
}
#[derive(Debug, Clone)]
pub struct Cond(pub Vec<Atom>);

pub fn parse_cond(src: &str) -> Result<Cond> {
    let t = tokens(src)?;
    let bad = || Error::new("invalid_expression", src);
    let mut atoms = Vec::new();
    let mut pos = 0;
    loop {
        let tok = t.get(pos).ok_or_else(bad)?.clone();
        pos += 1;
        let next = t.get(pos).map(String::as_str);
        let atom = if tok == "always" {
            Atom::Always
        } else if matches!(next, Some("==") | Some("!=")) {
            let neg = next == Some("!=");
            let lit = t.get(pos + 1).ok_or_else(bad)?;
            if !(lit.len() >= 2 && lit.starts_with('\'')) {
                return Err(bad());
            }
            pos += 2;
            Atom::Eq(tok, lit[1..lit.len() - 1].to_string(), neg)
        } else if next == Some("(") {
            pos += 1;
            let mut args = Vec::new();
            loop {
                args.push(t.get(pos).ok_or_else(bad)?.clone());
                pos += 1;
                match t.get(pos).map(String::as_str) {
                    Some(",") => pos += 1,
                    Some(")") => {
                        pos += 1;
                        break;
                    }
                    _ => return Err(bad()),
                }
            }
            match (tok.as_str(), args.as_slice()) {
                ("present", [a]) => Atom::Present(a.clone()),
                ("absent", [a]) => Atom::Absent(a.clone()),
                ("date_le", [a, b]) => Atom::DateLe(a.clone(), b.clone()),
                ("any_present", a) => Atom::AnyPresent(a.to_vec()),
                ("near", [a, b, t]) => Atom::Near(a.clone(), b.clone(), t.clone()),
                ("le", [a, b]) => Atom::Le(a.clone(), b.clone()),
                _ => return Err(Error::new("unknown_function", tok)),
            }
        } else {
            return Err(bad());
        };
        atoms.push(atom);
        match t.get(pos).map(String::as_str) {
            None => break,
            Some("and") => pos += 1,
            _ => return Err(bad()),
        }
    }
    Ok(Cond(atoms))
}
pub fn cond_refs(c: &Cond, out: &mut Vec<String>) {
    for a in &c.0 {
        match a {
            Atom::Always => {}
            Atom::Eq(a, ..) | Atom::Present(a) | Atom::Absent(a) => out.push(a.clone()),
            Atom::DateLe(a, b) => {
                out.push(a.clone());
                out.push(b.clone());
            }
            Atom::AnyPresent(v) => out.extend(v.iter().cloned()),
            Atom::Near(a, b, t) => out.extend([a, b, t].into_iter().filter(|x| !is_num_lit(x)).cloned()),
            Atom::Le(a, b) => out.extend([a, b].into_iter().filter(|x| !is_num_lit(x)).cloned()),
        }
    }
}
pub fn eval_cond(c: &Cond, get: &dyn Fn(&str) -> Option<String>) -> bool {
    c.0.iter().all(|a| match a {
        Atom::Always => true,
        Atom::Eq(id, lit, neg) => (get(id).unwrap_or_default() == *lit) != *neg,
        Atom::Present(id) => get(id).is_some_and(|v| !v.is_empty()),
        Atom::Absent(id) => get(id).is_none_or(|v| v.is_empty()),
        Atom::AnyPresent(v) => v.iter().any(|id| get(id).is_some_and(|x| !x.is_empty())),
        Atom::Near(a, b, t) => match (arg_num(a, get), arg_num(b, get), arg_num(t, get)) {
            (Some(x), Some(y), Some(t)) => (x - y).abs() <= t,
            _ => false,
        },
        Atom::Le(a, b) => match (arg_num(a, get), arg_num(b, get)) {
            (Some(x), Some(y)) => x <= y,
            _ => false,
        },
        Atom::DateLe(a, b) => match (get(a).and_then(|v| parse_date(&v)), get(b).and_then(|v| parse_date(&v))) {
            (Some(x), Some(y)) => x <= y,
            _ => false,
        },
    })
}

// ---- text formats ----
#[derive(Debug, Clone)]
pub enum Tok {
    Lit(String),
    Ph(String, Option<String>),
    Opt(Vec<Tok>),
}
pub fn parse_fmt(src: &str) -> Result<Vec<Tok>> {
    let bad = || Error::new("invalid_fmt", src);
    let mut stack: Vec<Vec<Tok>> = vec![Vec::new()];
    let cs: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut lit = String::new();
    macro_rules! flush {
        () => {
            if !lit.is_empty() {
                stack.last_mut().unwrap().push(Tok::Lit(std::mem::take(&mut lit)));
            }
        };
    }
    while i < cs.len() {
        match cs[i] {
            // `\[` `\]` `\{` `\}` `\\`: literal characters
            '\\' if i + 1 < cs.len() && "[]{}\\".contains(cs[i + 1]) => {
                lit.push(cs[i + 1]);
                i += 2;
            }
            '{' => {
                flush!();
                let j = cs[i..].iter().position(|c| *c == '}').ok_or_else(bad)? + i;
                let body: String = cs[i + 1..j].iter().collect();
                let (id, f) = match body.split_once('|') {
                    Some((a, b)) => (a.to_string(), Some(b.to_string())),
                    None => (body, None),
                };
                if id.is_empty() {
                    return Err(bad());
                }
                stack.last_mut().unwrap().push(Tok::Ph(id, f));
                i = j + 1;
            }
            '[' => {
                flush!();
                if stack.len() > 1 {
                    return Err(bad());
                }
                stack.push(Vec::new());
                i += 1;
            }
            ']' => {
                flush!();
                if stack.len() != 2 {
                    return Err(bad());
                }
                let inner = stack.pop().unwrap();
                stack.last_mut().unwrap().push(Tok::Opt(inner));
                i += 1;
            }
            c => {
                lit.push(c);
                i += 1;
            }
        }
    }
    flush!();
    if stack.len() != 1 {
        return Err(bad());
    }
    Ok(stack.pop().unwrap())
}
#[cfg(test)]
#[test]
fn fmt_escapes_are_literal() {
    let t = parse_fmt(r" \[코드 {c}\] [({o})]").unwrap();
    let get = |id: &str| if id == "c" { Some("7".to_string()) } else { None };
    assert_eq!(render_fmt(&t, &get).unwrap().0, " [코드 7] ");
}
pub fn fmt_refs(t: &[Tok], out: &mut Vec<(String, Option<String>)>) {
    for x in t {
        match x {
            Tok::Lit(_) => {}
            Tok::Ph(i, f) => out.push((i.clone(), f.clone())),
            Tok::Opt(v) => fmt_refs(v, out),
        }
    }
}
pub const FILTERS: [&str; 15] = ["nobullet", "dot", "dash", "usd", "krw", "date_kr", "date_kr0", "date_dot", "date_dot_wd", "md_wd", "pct", "usd2", "num", "ganada", "attach"];

/// Hangul item markers of an official letter's second level (가. 나. … 하.), then 거. 너. ….
const GANADA: [&str; 28] = [
    "가", "나", "다", "라", "마", "바", "사", "아", "자", "차", "카", "타", "파", "하", "거", "너", "더", "러", "머", "버", "서", "어", "저", "처", "커", "터", "퍼", "허",
];
/// A line's own item marker written by the LLM ("1.", "2)", "가.", "나)", "①", "- ", "∙ ") is
/// dropped: numbering is the core's (D4-3). A marker must be followed by whitespace, and a number
/// marker has one or two digits, so "1.5배", "2026. 3. 2." and "-5%" stay as written (D4-2).
fn strip_marker(l: &str) -> &str {
    let t = l.trim_start();
    let mut cs = t.char_indices();
    let Some((_, c)) = cs.next() else { return t };
    // bullets and circled numbers: the symbol, then whitespace
    if "-∙•·*".contains(c) || ('①'..='⑳').contains(&c) {
        let rest = &t[c.len_utf8()..];
        return if rest.starts_with(char::is_whitespace) { rest.trim_start() } else { t };
    }
    let end = if c.is_ascii_digit() {
        let n = t.chars().take_while(|c| c.is_ascii_digit()).count();
        (n <= 2).then_some(n)
    } else if GANADA.iter().any(|g| g.starts_with(c)) {
        Some(c.len_utf8())
    } else {
        None
    };
    if let Some(i) = end {
        let rest = &t[i..];
        if let Some(r) = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')')) {
            if r.starts_with(char::is_whitespace) {
                let r = r.trim_start();
                // "3. 15.(금)", "12. 1. 중간보고": a month-day date, not a marker. A number the LLM
                // wrote before such a date then shows twice, which is visible; a dropped month is not.
                if c.is_ascii_digit() && starts_with_day(r) {
                    return t;
                }
                return r;
            }
        }
    }
    t
}
/// `\d{1,2}\.` followed by whitespace, `(` or the end.
fn starts_with_day(s: &str) -> bool {
    let n = s.chars().take_while(|c| c.is_ascii_digit()).count();
    (1..=2).contains(&n) && s[n..].strip_prefix('.').is_some_and(|r| r.is_empty() || r.starts_with(char::is_whitespace) || r.starts_with('('))
}

pub fn group_digits(s: &str) -> String {
    let (neg, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let (int, frac) = match rest.split_once('.') {
        Some((a, b)) => (a, Some(b)),
        None => (rest, None),
    };
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{}{}{}", if neg { "-" } else { "" }, out, frac.map(|f| format!(".{f}")).unwrap_or_default())
}
fn apply_filter(v: &str, f: Option<&str>) -> String {
    match f {
        None => v.to_string(),
        Some("usd") | Some("krw") => group_digits(v),
        Some("nobullet") => v
            .split('\n')
            .map(|l| l.trim_start().trim_start_matches(['-', '∙', '•', '·', '*']).trim_start().to_string())
            .collect::<Vec<_>>()
            .join("\n"),
        Some(k @ ("dot" | "dash")) => v
            .split('\n')
            .map(|l| format!("{} {}", if k == "dot" { "∙" } else { "-" }, l.trim_start().trim_start_matches(['-', '∙', '•', '·', '*']).trim_start()))
            .collect::<Vec<_>>()
            .join("\n"),
        // official-letter numbering: one item per line, the core writes the markers
        Some("num") => v.split('\n').enumerate().map(|(i, l)| format!("{}. {}", i + 1, strip_marker(l))).collect::<Vec<_>>().join("\n"),
        Some("ganada") => v
            .split('\n')
            .enumerate()
            .map(|(i, l)| match GANADA.get(i) {
                Some(g) => format!("  {g}. {}", strip_marker(l)),
                None => format!("  {}. {}", i + 1, strip_marker(l)),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        // 붙임: one item stays bare; several are numbered with the continuation lines aligned under the first
        Some("attach") => {
            let ls: Vec<&str> = v.split('\n').map(strip_marker).collect();
            if ls.len() == 1 {
                ls[0].to_string()
            } else {
                ls.iter().enumerate().map(|(i, l)| format!("{}{}. {l}", if i == 0 { "" } else { "      " }, i + 1)).collect::<Vec<_>>().join("\n")
            }
        }
        Some("pct") => parse_decimal(v).map(|d| format!("{}%", (d * Decimal::ONE_HUNDRED).normalize())).unwrap_or_else(|| v.to_string()),
        Some("usd2") => parse_decimal(v).map(|d| group_digits(&d.round_dp(2).normalize().to_string())).unwrap_or_else(|| v.to_string()),
        Some(k @ ("date_kr" | "date_kr0" | "date_dot" | "date_dot_wd" | "md_wd")) => match parse_date(v) {
            Some((y, m, d)) => match k {
                "date_kr" => format!("{y}년 {m}월 {d}일"),
                "date_kr0" => format!("{y}년 {m:02}월 {d:02}일"),
                "date_dot_wd" => format!("{y}.{m:02}.{d:02}({})", weekday_kr(v).unwrap_or("?")),
                "md_wd" => format!("{m}.{d}({})", weekday_kr(v).unwrap_or("?")),
                _ => format!("{y}.{m:02}.{d:02}"),
            },
            None => v.to_string(),
        },
        Some(_) => v.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub slot: String,
    pub start: usize,
    pub end: usize,
}
/// None when a non-optional placeholder is unresolved. Spans are char offsets.
pub fn render_fmt(t: &[Tok], get: &dyn Fn(&str) -> Option<String>) -> Option<(String, Vec<Span>)> {
    fn run(t: &[Tok], get: &dyn Fn(&str) -> Option<String>, text: &mut String, n: &mut usize, spans: &mut Vec<Span>) -> bool {
        for x in t {
            match x {
                Tok::Lit(s) => {
                    text.push_str(s);
                    *n += s.chars().count();
                }
                Tok::Ph(id, f) => {
                    let Some(v) = get(id).filter(|v| !v.is_empty()) else { return false };
                    let shown = apply_filter(&v, f.as_deref());
                    let len = shown.chars().count();
                    spans.push(Span { slot: id.clone(), start: *n, end: *n + len });
                    text.push_str(&shown);
                    *n += len;
                }
                Tok::Opt(inner) => {
                    let (mut t2, mut n2, mut s2) = (String::new(), *n, Vec::new());
                    if run(inner, get, &mut t2, &mut n2, &mut s2) {
                        text.push_str(&t2);
                        *n = n2;
                        spans.extend(s2);
                    }
                }
            }
        }
        true
    }
    let (mut text, mut n, mut spans) = (String::new(), 0, Vec::new());
    run(t, get, &mut text, &mut n, &mut spans).then_some((text, spans))
}

// ---- dates ----
/// Days since 1970-01-01 (proleptic Gregorian).
pub fn epoch_day(s: &str) -> Option<i64> {
    let (y, m, d) = parse_date(s)?;
    let (y, m, d) = (y as i64, m as i64, d as i64);
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}
pub fn from_epoch_day(z: i64) -> String {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}
pub fn weekday_kr(s: &str) -> Option<&'static str> {
    // 1970-01-01 was a Thursday
    let w = (epoch_day(s)? + 3).rem_euclid(7);
    Some(["월", "화", "수", "목", "금", "토", "일"][w as usize])
}

// ---- typed evaluation (values are strings; numbers are Decimal text, dates YYYY-MM-DD,
// lists are JSON arrays). None when an input is unresolved or a lookup fails. ----
pub struct Ctx<'a> {
    pub get: &'a dyn Fn(&str) -> Option<String>,
    pub reg: Option<&'a crate::regulation::Regulation>,
}

fn num(s: &str) -> Option<Decimal> {
    parse_decimal(s)
}
fn list(s: &str) -> Option<Vec<serde_json::Map<String, serde_json::Value>>> {
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    v.as_array()?.iter().map(|x| x.as_object().cloned()).collect()
}
fn field(o: &serde_json::Map<String, serde_json::Value>, k: &str) -> Option<String> {
    match o.get(k)? {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}
fn dstr(d: Decimal) -> String {
    d.normalize().to_string()
}

/// Segments as `[{country, city, start, end}]` with per-segment nights.
fn seg_rows(s: &str) -> Option<Vec<(String, String, String, String, i64)>> {
    let l = list(s)?;
    let mut out = Vec::new();
    for o in &l {
        let (c, ci, a, b) = (field(o, "country")?, field(o, "city").unwrap_or_default(), field(o, "start")?, field(o, "end")?);
        let n = epoch_day(&b)? - epoch_day(&a)?;
        if n < 0 {
            return None;
        }
        out.push((c, ci, a, b, n));
    }
    (!out.is_empty()).then_some(out)
}

pub fn round_to(x: Decimal, unit: Decimal, mode: &str) -> Option<Decimal> {
    if unit <= Decimal::ZERO {
        return None;
    }
    let st = match mode {
        "half_up" => RoundingStrategy::MidpointAwayFromZero,
        "floor" => RoundingStrategy::ToNegativeInfinity,
        "ceil" => RoundingStrategy::ToPositiveInfinity,
        _ => return None,
    };
    x.checked_div(unit)?.round_dp_with_strategy(0, st).checked_mul(unit)
}
fn int(d: Decimal) -> Option<i128> {
    (d.fract().is_zero()).then(|| d.trunc().mantissa() / 10i128.pow(d.trunc().scale()))
}
fn days_in(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 { 29 } else { 28 },
    }
}
/// Calendar months; a day past the end of the target month becomes its last day (01-31 + 1 = 02-28/29).
pub fn add_months(date: &str, n: i64) -> Option<String> {
    let (y, m, d) = parse_date(date)?;
    let k = y as i64 * 12 + (m as i64 - 1) + n;
    let (y2, m2) = (k.div_euclid(12), k.rem_euclid(12) + 1);
    if !(1..=9999).contains(&y2) {
        return None;
    }
    Some(format!("{y2:04}-{m2:02}-{:02}", (d as i64).min(days_in(y2, m2))))
}
/// Whole months from a to b: the largest m with add_months(a, m) <= b. None if b < a.
pub fn months_between(a: &str, b: &str) -> Option<i64> {
    let (ea, eb) = (epoch_day(a)?, epoch_day(b)?);
    if eb < ea {
        return None;
    }
    let ((ya, ma, _), (yb, mb, _)) = (parse_date(a)?, parse_date(b)?);
    let mut m = (yb as i64 - ya as i64) * 12 + (mb as i64 - ma as i64);
    while m > 0 && epoch_day(&add_months(a, m)?)? > eb {
        m -= 1;
    }
    Some(m)
}

pub fn eval_val(e: &Expr, cx: &Ctx) -> Option<String> {
    let (f, args) = match e {
        Expr::Num(d) => return Some(d.to_string()),
        Expr::Str(s) => return Some(s.clone()),
        Expr::Ref(r) => return (cx.get)(r),
        Expr::Call(f, a) => (f.as_str(), a),
    };
    // coalesce tolerates unresolved arguments
    if f == "coalesce" {
        return args.iter().find_map(|a| eval_val(a, cx).filter(|v| !v.is_empty()));
    }
    // segs(segments, country, city, start, end): the segments list when given, else one segment
    if f == "segs" && args.len() == 5 {
        if let Some(l) = eval_val(&args[0], cx).filter(|x| list(x).is_some_and(|l| !l.is_empty())) {
            return Some(l);
        }
        // city may be empty (grade then comes from the country)
        let v: Vec<String> = args[1..].iter().enumerate().map(|(k, a)| if k == 1 { Some(eval_val(a, cx).unwrap_or_default()) } else { eval_val(a, cx) }).collect::<Option<_>>()?;
        return Some(serde_json::json!([{"country": v[0], "city": v[1], "start": v[2], "end": v[3]}]).to_string());
    }
    let v: Vec<String> = args.iter().map(|a| eval_val(a, cx)).collect::<Option<_>>()?;
    let n = |i: usize| v.get(i).and_then(|s| num(s));
    let reg = cx.reg;
    let r = match (f, v.len()) {
        ("sum", k) if k > 0 => {
            let mut a = Decimal::ZERO;
            for x in &v {
                a = a.checked_add(num(x)?)?;
            }
            a.to_string()
        }
        ("mul", 2) => n(0)?.checked_mul(n(1)?)?.to_string(),
        ("fx", 2) => round_krw(n(0)?.checked_mul(n(1)?)?).to_string(),
        ("vat", 1) => round_krw(n(0)?.checked_mul(Decimal::new(1, 1))?).to_string(),
        ("round_krw", 1) => round_krw(n(0)?).to_string(),
        ("sub", 2) => n(0)?.checked_sub(n(1)?)?.to_string(),
        ("div", 2) => n(0)?.checked_div(n(1)?)?.to_string(),
        ("neg", 1) => (-n(0)?).to_string(),
        ("abs", 1) => n(0)?.abs().to_string(),
        ("pos", 1) => (if n(0)? > Decimal::ZERO { "true" } else { "false" }).to_string(),
        ("if_pos", 3) => if n(0)? > Decimal::ZERO { v[1].clone() } else { v[2].clone() },
        ("date_add", 2) => from_epoch_day(epoch_day(&v[0])? + i64::try_from(n(1)?.trunc().mantissa()).ok()?),
        ("days", 2) => (epoch_day(&v[1])? - epoch_day(&v[0])?).to_string(),
        ("weekday", 1) => weekday_kr(&v[0])?.to_string(),
        ("grade", 1) => reg?.grade(&v[0])?.0,
        ("grade_basis", 1) => reg?.grade(&v[0])?.1,
        ("rate", 3) => reg?.rate(&v[0], &v[1], &v[2])?.to_string(),
        ("discount", 1) => dstr(reg?.discount(u32::try_from(n(0)?.trunc().mantissa()).ok()?)),
        ("reg_status", 0) => reg?.status.clone(),
        ("discount_status", 0) => reg?.discount_status.clone(),
        ("nights", 1) => seg_rows(&v[0])?.iter().map(|s| s.4).sum::<i64>().to_string(),
        // USD before discount. Meals: nights per segment, the last segment +1 (return day).
        ("per_diem", 3) => {
            let reg = reg?;
            let rows = seg_rows(&v[1])?;
            let last = rows.len() - 1;
            let mut t = Decimal::ZERO;
            for (i, (c, ci, _, _, nn)) in rows.iter().enumerate() {
                let g = reg.place_grade(c, ci)?.0;
                let units = if (v[0] == "meal" || v[0] == "daily") && i == last { nn + 1 } else { *nn };
                t += reg.rate(&v[0], &g, &v[2])? * Decimal::from(units);
            }
            t.to_string()
        }
        ("per_diem_note", 4) => {
            let reg = reg?;
            let rows = seg_rows(&v[1])?;
            let last = rows.len() - 1;
            let mut parts: Vec<String> = Vec::new();
            for (i, (c, ci, _, _, nn)) in rows.iter().enumerate() {
                let g = reg.place_grade(c, ci)?.0;
                let c = if reg.city_grade(c, ci).is_some() { format!("{c}({})", reg.city_name(ci)) } else { c.clone() };
                let r = reg.rate(&v[0], &g, &v[2])?;
                let (units, unit) = if v[0] == "meal" || v[0] == "daily" { (if i == last { nn + 1 } else { *nn }, "일") } else { (*nn, "박") };
                parts.push(format!("{c} {g}(일 {}$) {units}{unit}", dstr(r)));
            }
            let d = num(&v[3])?;
            if d != Decimal::ONE {
                parts.push(format!("{}% 적용", dstr(d * Decimal::ONE_HUNDRED)));
            }
            parts.join(", ")
        }
        ("grades", 1) => {
            let reg = reg?;
            let mut gs: Vec<String> = Vec::new();
            for (c, ci, ..) in seg_rows(&v[0])? {
                let g = reg.place_grade(&c, &ci)?.0;
                let c = if reg.city_grade(&c, &ci).is_some() { format!("{c}({})", reg.city_name(&ci)) } else { c };
                let item = format!("{c} {g}");
                if !gs.contains(&item) {
                    gs.push(item);
                }
            }
            gs.join(", ")
        }
        // `ok`, or what is wrong (unlisted place mapped by alias, unknown place)
        ("grades_ok", 1) => {
            let reg = reg?;
            let mut bad: Vec<String> = Vec::new();
            for (c, ci, ..) in seg_rows(&v[0])? {
                if let Some(m) = reg.city_mismatch(&c, &ci) {
                    bad.push(m);
                }
                match reg.place_grade(&c, &ci) {
                    Some((_, b)) if b == "listed" => {}
                    Some((g, b)) => bad.push(format!("{c}→{g} ({b})")),
                    None => bad.push(format!("{c}: 등급표에 없음")),
                }
            }
            if bad.is_empty() { "ok".into() } else { bad.join("; ") }
        }
        // items: [{category, desc, unit, count, currency}] -> KRW total for a category
        ("items_krw", 3) => {
            let fx = num(&v[2]);
            let mut t = Decimal::ZERO;
            for o in list(&v[0])?.iter().filter(|o| field(o, "category").as_deref() == Some(v[1].as_str())) {
                let amt = num(&field(o, "unit")?)? * num(&field(o, "count").unwrap_or_else(|| "1".into()))?;
                t += if field(o, "currency").as_deref() == Some("KRW") { amt } else { amt * fx? };
            }
            round_krw(t).to_string()
        }
        ("items_usd", 2) => {
            let mut t = Decimal::ZERO;
            for o in list(&v[0])?.iter().filter(|o| field(o, "category").as_deref() == Some(v[1].as_str()) && field(o, "currency").as_deref() != Some("KRW")) {
                t += num(&field(o, "unit")?)? * num(&field(o, "count").unwrap_or_else(|| "1".into()))?;
            }
            t.to_string()
        }
        ("items_note", 2) => {
            let mut parts: Vec<String> = Vec::new();
            for o in list(&v[0])?.iter().filter(|o| field(o, "category").as_deref() == Some(v[1].as_str())) {
                let desc = field(o, "desc").unwrap_or_default();
                let unit = num(&field(o, "unit")?)?;
                let cnt = num(&field(o, "count").unwrap_or_else(|| "1".into()))?;
                let money = if field(o, "currency").as_deref() == Some("KRW") { format!("{}원", group_digits(&dstr(unit))) } else { format!("{}$", group_digits(&dstr(unit))) };
                parts.push(if cnt == Decimal::ONE { format!("{desc} {money}") } else { format!("{desc} {money} × {}회", dstr(cnt)) });
            }
            parts.join("\n")
        }
        // itinerary [{start, end, ...}]: first starts on trip start, last ends on trip end,
        // each row ends on or after it starts, the next row starts the same day or the day after.
        ("itin_check", 3) => {
            let l = list(&v[0])?;
            if l.is_empty() {
                return Some("일정이 비어 있음".into());
            }
            let (ts, te) = (epoch_day(&v[1])?, epoch_day(&v[2])?);
            let mut prob: Vec<String> = Vec::new();
            let mut prev_end: Option<i64> = None;
            for (i, o) in l.iter().enumerate() {
                let (a, b) = (field(o, "start").and_then(|x| epoch_day(&x)), field(o, "end").and_then(|x| epoch_day(&x)));
                let (Some(a), Some(b)) = (a, b) else {
                    prob.push(format!("{}행 날짜 형식", i + 1));
                    continue;
                };
                if b < a {
                    prob.push(format!("{}행 종료가 시작보다 앞섬", i + 1));
                }
                if let Some(p) = prev_end {
                    if a > p + 1 {
                        prob.push(format!("{}행 앞에 {}일 공백", i + 1, a - p - 1));
                    } else if a < p {
                        prob.push(format!("{}행이 앞 행과 겹침", i + 1));
                    }
                }
                prev_end = Some(b);
            }
            let first = field(&l[0], "start").and_then(|x| epoch_day(&x));
            if first != Some(ts) {
                prob.push("첫 행 시작이 출장 시작일과 다름".into());
            }
            if prev_end != Some(te) {
                prob.push("마지막 행 종료가 출장 종료일과 다름".into());
            }
            if prob.is_empty() { "ok".into() } else { prob.join("; ") }
        }
        ("seg_join", 2) => {
            let mut seen: Vec<String> = Vec::new();
            for o in list(&v[0])? {
                if let Some(x) = field(&o, &v[1]).filter(|x| !x.is_empty()) {
                    if !seen.contains(&x) {
                        seen.push(x);
                    }
                }
            }
            seen.join(", ")
        }
        // segments vs trip period: first start = trip start, last end = trip end, no gap/overlap
        ("segs_check", 3) => {
            let Some(rows) = seg_rows(&v[0]) else { return Some("구간 날짜가 잘못됐다(종료일이 시작일보다 앞섬)".into()) };
            let mut prob: Vec<String> = Vec::new();
            if rows[0].2 != v[1] {
                prob.push(format!("첫 구간 시작 {} ≠ 출장 시작 {}", rows[0].2, v[1]));
            }
            if rows[rows.len() - 1].3 != v[2] {
                prob.push(format!("마지막 구간 종료 {} ≠ 출장 종료 {}", rows[rows.len() - 1].3, v[2]));
            }
            for w in rows.windows(2) {
                if w[0].3 != w[1].2 {
                    prob.push(format!("구간 경계 {} → {} 불연속(앞 구간 끝 = 뒤 구간 시작)", w[0].3, w[1].2));
                }
            }
            if prob.is_empty() { "ok".into() } else { prob.join("; ") }
        }
        // round_to(x, unit, mode): to a multiple of unit; mode half_up (away from zero at .5) | floor | ceil
        ("round_to", 3) => dstr(round_to(n(0)?, n(1)?, &v[2])?),
        // pct(x, p): p percent of x, unrounded
        ("pct", 2) => dstr(n(0)?.checked_mul(n(1)?)?.checked_div(Decimal::ONE_HUNDRED)?),
        // share(total, p, unit, mode): one party's p% of total, rounded; share_rest gives the
        // other party total - share, so the parts always add up to the total
        ("share", 4) => dstr(round_to(n(0)?.checked_mul(n(1)?)?.checked_div(Decimal::ONE_HUNDRED)?, n(2)?, &v[3])?),
        ("share_rest", 4) => dstr(n(0)?.checked_sub(round_to(n(0)?.checked_mul(n(1)?)?.checked_div(Decimal::ONE_HUNDRED)?, n(2)?, &v[3])?)?),
        // prorate(amount, annual rate %, months) = amount x rate/100 x months/12, unrounded
        ("prorate", 3) => dstr(n(0)?.checked_mul(n(1)?)?.checked_mul(n(2)?)?.checked_div(Decimal::from(1200))?),
        ("add_months", 2) => add_months(&v[0], i64::try_from(int(n(1)?)?).ok()?)?,
        ("months_between", 2) => months_between(&v[0], &v[1])?.to_string(),
        // list_sum(list, field): sum of a numeric field over every item (all items must have it)
        ("list_sum", 2) => {
            let mut t = Decimal::ZERO;
            for o in list(&v[0])? {
                t = t.checked_add(num(&field(&o, &v[1])?)?)?;
            }
            dstr(t)
        }
        ("between", 3) => (if n(1)? <= n(0)? && n(0)? <= n(2)? { "true" } else { "false" }).to_string(),
        ("if_eq", 4) => if v[0].trim() == v[1].trim() { v[2].clone() } else { v[3].clone() },
        ("list_len", 1) => list(&v[0])?.len().to_string(),
        _ => return None,
    };
    Some(r)
}

#[cfg(test)]
mod calc_n_tests {
    //! N1~N3 functions and N10 rule atoms. Expected values are worked out by hand (comments),
    //! never with the functions under test.
    use super::*;
    fn ev(src: &str) -> Option<String> {
        let get = |id: &str| -> Option<String> {
            match id {
                "lst" => Some(r#"[{"p":"28000"},{"p":"2,000"}]"#.into()),
                "lst_bad" => Some(r#"[{"p":"28000"},{"q":"1"}]"#.into()),
                _ => None,
            }
        };
        eval_val(&parse_expr(src).unwrap(), &Ctx { get: &get, reg: None })
    }
    fn is(src: &str, want: &str) {
        assert_eq!(ev(src).as_deref(), Some(want), "{src}");
    }
    #[test]
    fn round_to_modes_and_bounds() {
        is("round_to(802172.8, 1, 'half_up')", "802173");
        is("round_to(2.5, 1, 'half_up')", "3");
        is("round_to(neg(2.5), 1, 'half_up')", "-3"); // away from zero
        is("round_to(neg(2.5), 1, 'floor')", "-3");
        is("round_to(neg(2.5), 1, 'ceil')", "-2");
        is("round_to(2.5, 1, 'floor')", "2");
        is("round_to(2.1, 1, 'ceil')", "3");
        is("round_to(29205000, 1000000, 'half_up')", "29000000");
        is("round_to(15795000, 10000, 'half_up')", "15800000"); // 1579.5 -> 1580
        is("round_to(15795000, 10000, 'floor')", "15790000");
        is("round_to(29.205, 0.01, 'half_up')", "29.21");
        is("round_to(0, 1000, 'ceil')", "0");
        assert_eq!(ev("round_to(1, 0, 'half_up')"), None);
        assert_eq!(ev("round_to(1, neg(1), 'half_up')"), None);
        assert_eq!(ev("round_to(1, 1, 'bankers')"), None);
        assert_eq!(ev("round_to(missing, 1, 'half_up')"), None);
    }
    #[test]
    fn pct_share_prorate() {
        is("pct(3646240, 15)", "546936");
        is("pct(546936, 110)", "601629.6");
        is("pct(100, 0)", "0");
        is("share(4807, 60, 1, 'half_up')", "2884"); // 2884.2
        is("share_rest(4807, 60, 1, 'half_up')", "1923");
        is("share(30000, 64.9, 1, 'half_up')", "19470");
        is("share_rest(30000, 64.9, 1, 'half_up')", "10530");
        is("share(5, 50, 1, 'half_up')", "3"); // 2.5 -> 3, rest 2
        is("share_rest(5, 50, 1, 'half_up')", "2");
        is("share(5, 50, 1, 'floor')", "2");
        is("share_rest(5, 50, 1, 'floor')", "3");
        is("share(4807, 100, 1, 'half_up')", "4807");
        is("share_rest(4807, 100, 1, 'half_up')", "0");
        is("prorate(601630, 0.15, 13)", "977.64875"); // 902.445 x 13 / 12
        is("round_to(prorate(601630, 0.15, 13), 1, 'half_up')", "978");
        is("prorate(546936, 0.7, 12)", "3828.552");
        is("prorate(1000, 12, 0)", "0");
    }
    #[test]
    fn months() {
        is("add_months('2026-01-31', 1)", "2026-02-28");
        is("add_months('2024-01-31', 1)", "2024-02-29");
        is("add_months('2026-01-23', 13)", "2027-02-23");
        is("add_months('2026-03-31', neg(1))", "2026-02-28");
        is("add_months('2026-12-15', 1)", "2027-01-15");
        is("add_months('2026-12-15', 0)", "2026-12-15");
        assert_eq!(ev("add_months('2026-12-15', 1.5)"), None);
        assert_eq!(ev("add_months('2026-02-30', 1)"), None);
        is("months_between('2026-01-23', '2030-03-22')", "49");
        is("months_between('2026-01-23', '2030-03-23')", "50");
        is("months_between('2026-01-31', '2026-02-28')", "1");
        is("months_between('2026-02-28', '2026-03-27')", "0");
        is("months_between('2026-05-05', '2026-05-05')", "0");
        assert_eq!(ev("months_between('2026-05-06', '2026-05-05')"), None);
    }
    #[test]
    fn list_sum_all_items() {
        is("list_sum(lst, 'p')", "30000");
        assert_eq!(ev("list_sum(lst_bad, 'p')"), None);
    }
    #[test]
    fn near_and_le_atoms() {
        let get = |id: &str| -> Option<String> {
            match id {
                "a" => Some("4807".into()),
                "b" => Some("4806".into()),
                "tol" => Some("0.5".into()),
                _ => None,
            }
        };
        let c = |s: &str| eval_cond(&parse_cond(s).unwrap(), &get);
        assert!(c("near(a, b, 1)"));
        assert!(!c("near(a, b, 0)"));
        assert!(!c("near(a, b, tol)"));
        assert!(c("near(a, 4807, 0)"));
        assert!(!c("near(a, missing, 100)")); // unresolved is not near
        assert!(c("le(b, a)"));
        assert!(!c("le(a, b)"));
        assert!(c("le(a, a)"));
        assert!(!c("le(a, missing)"));
        let mut refs = Vec::new();
        cond_refs(&parse_cond("near(a, 4807, 1) and le(b, 10)").unwrap(), &mut refs);
        assert_eq!(refs, vec!["a".to_string(), "b".to_string()]);
    }
}

#[cfg(test)]
#[test]
fn letter_numbering_filters() {
    // expected strings written by hand
    assert_eq!(apply_filter("관련: 계약서\n2. 아래와 같이 보고합니다.", Some("num")), "1. 관련: 계약서\n2. 아래와 같이 보고합니다.");
    assert_eq!(apply_filter("가. 일정\n나) 인력\n- 장비\n1.5배 증원", Some("ganada")), "  가. 일정\n  나. 인력\n  다. 장비\n  라. 1.5배 증원");
    // a word starting with a marker syllable is not a marker
    assert_eq!(apply_filter("나는 간다", Some("ganada")), "  가. 나는 간다");
    assert_eq!(apply_filter("월간 보고서 1부.", Some("attach")), "월간 보고서 1부.");
    assert_eq!(apply_filter("1. 보고서 1부.\n일정표 1부.", Some("attach")), "1. 보고서 1부.\n      2. 일정표 1부.");
    // dates and signs are content, not markers (reviewer #3)
    assert_eq!(apply_filter("2026. 3. 2. 착수보고회 결과를 보고합니다.", Some("num")), "1. 2026. 3. 2. 착수보고회 결과를 보고합니다.");
    assert_eq!(apply_filter("-5% 감소 추세", Some("num")), "1. -5% 감소 추세");
    // month-day dates at the start of a line (reviewer round 2)
    assert_eq!(apply_filter("3. 15.(금) 착수보고회 개최\n12. 1. 중간보고\n가. 3. 2.", Some("ganada")), "  가. 3. 15.(금) 착수보고회 개최\n  나. 12. 1. 중간보고\n  다. 3. 2.");
    assert_eq!(apply_filter("3. 2.(월) 회의 결과를 보고합니다.\n2. 3월 계획", Some("num")), "1. 3. 2.(월) 회의 결과를 보고합니다.\n2. 3월 계획");
    assert_eq!(apply_filter("12. 열두째\n① 원문자\n- 기호", Some("num")), "1. 열두째\n2. 원문자\n3. 기호");
    assert_eq!(apply_filter("∙\n가.나", Some("ganada")), "  가. ∙\n  나. 가.나");
    let many = (0..30).map(|i| format!("x{i}")).collect::<Vec<_>>().join("\n");
    let out = apply_filter(&many, Some("ganada"));
    assert!(out.starts_with("  가. x0") && out.contains("  허. x27") && out.ends_with("  30. x29"));
}
