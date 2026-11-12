//! Paragraph numbering check (generic, no document-type rules).
//!
//! Hancom shows the number of an auto-numbered paragraph (paraPr heading NUMBER) in front of its
//! text. If the text also starts with a typed number ("1. "), the page shows "1. 1. …". The same
//! doubling happens when typed text carries two markers ("1. 1. …", "1. 가. …"), on any line of a
//! paragraph. Two different one/two-digit numbers in a row ("3. 2. 회의") are read as a date.
//! This module reads the saved header/section XML only: paraPr headings, numbering paraHead format
//! text and the paragraph's own text runs. It does not compute counter values.

use std::collections::HashMap;
use std::io::{Cursor, Read};

/// A paragraph Hancom numbers or bullets by itself.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoPara {
    pub part: String,
    pub text: String,
    /// "NUMBER" or "BULLET"
    pub kind: String,
    /// numbering format text of that level (e.g. "^1."), bullet char for bullets
    pub format: Option<String>,
}

/// A paragraph whose number would show twice.
#[derive(Debug, Clone, PartialEq)]
pub struct NumberingIssue {
    pub part: String,
    pub text: String,
    /// "auto_and_typed": auto-numbered paragraph whose text starts with a typed number;
    /// "typed_twice": a line starts with two typed markers;
    /// "maybe_date": the two markers are the same number in date range ("1. 1.", "11. 11."),
    /// which reads as a doubled number or as a date (Jan 1, Nov 11);
    /// "auto_date": auto-numbered paragraph whose text starts with two different numbers in date
    /// range ("3. 2. 회의", "1. 2. 항목"), which reads as a date or as a doubled number.
    pub reason: &'static str,
    /// true: drafting stops (numbering_doubled). false: reported as a warning only (ambiguous).
    pub blocking: bool,
}

const GANADA: &str = "가나다라마바사아자차카타파하";

/// Byte length of a typed list marker at the start of `s` (leading spaces and the space after the
/// marker included), or None. Markers: `1.` `1)` `(1)` `가.` `가)` `(가)` `①`. Numbers are one or two
/// digits so years ("2026. 1. 1.") are not taken for markers; a marker must be followed by a space
/// (circled numbers may be followed directly by text).
pub fn typed_marker_len(s: &str) -> Option<usize> {
    let lead = s.len() - s.trim_start().len();
    let t = &s[lead..];
    let mut cs = t.char_indices().peekable();
    let (_, c0) = *cs.peek()?;
    let body_end = if ('①'..='⑳').contains(&c0) {
        let e = c0.len_utf8();
        let ws = t[e..].len() - t[e..].trim_start().len();
        return Some(lead + e + ws);
    } else if c0 == '(' {
        let inner = &t[1..];
        let n = inner.chars().take_while(|c| c.is_ascii_digit()).count();
        let k = if (1..=2).contains(&n) { n } else if inner.chars().next().is_some_and(|c| GANADA.contains(c)) { inner.chars().next()?.len_utf8() } else { return None };
        if !inner[k..].starts_with(')') {
            return None;
        }
        1 + k + 1
    } else if c0.is_ascii_digit() {
        let n = t.chars().take_while(|c| c.is_ascii_digit()).count();
        if n > 2 || !(t[n..].starts_with('.') || t[n..].starts_with(')')) {
            return None;
        }
        n + 1
    } else if GANADA.contains(c0) {
        let k = c0.len_utf8();
        if !(t[k..].starts_with('.') || t[k..].starts_with(')')) {
            return None;
        }
        k + 1
    } else {
        return None;
    };
    let rest = &t[body_end..];
    let ws = rest.len() - rest.trim_start().len();
    if ws == 0 && !rest.is_empty() {
        return None;
    }
    Some(lead + body_end + ws)
}

/// The number of a typed `N.` marker (dot only: dates are written with dots) at the start,
/// followed by a space or the end.
fn dot_number(s: &str) -> Option<(u32, usize)> {
    let lead = s.len() - s.trim_start().len();
    let t = &s[lead..];
    let n = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if !(1..=2).contains(&n) || !t[n..].starts_with('.') {
        return None;
    }
    let rest = &t[n + 1..];
    // "2.(월)": a weekday right after the day
    if !(rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with('(')) {
        return None;
    }
    let ws = rest.len() - rest.trim_start().len();
    Some((t[..n].parse().ok()?, lead + n + 1 + ws))
}

/// How the first two markers of a line read as a date: None = not a date, Some(true) = a date
/// (two different numbers in month/day or yy/mm range: "3. 2. 회의", "26. 11. 9."),
/// Some(false) = the same number twice in date range ("1. 1.", "11. 11."), which may be either.
fn date_reading(s: &str) -> Option<bool> {
    let (a, k) = dot_number(s)?;
    let (b, _) = dot_number(&s[k..])?;
    let in_range = (1..=12).contains(&a) && (1..=31).contains(&b) || (13..=99).contains(&a) && (1..=12).contains(&b);
    in_range.then_some(a != b)
}

/// The line starts with a date-like pair of numbers ("3. 2. 회의", "1. 1. 신정").
pub fn starts_with_date(s: &str) -> bool {
    date_reading(s).is_some()
}

/// A typed marker at the start of an auto-numbered paragraph's first line: the spaced markers of
/// `typed_marker_len`, three-digit numbers ("100. 항목"), and markers glued to a letter
/// ("1.항목", "1)Item", "가.항목"). Not "1.5배" (a digit follows) and not years ("2026.").
fn auto_marker(s: &str) -> bool {
    if typed_marker_len(s).is_some() {
        return true;
    }
    let t = s.trim_start();
    let letter = |r: &str| r.chars().next().is_some_and(|c| c.is_alphabetic());
    let spaced = |r: &str| r.is_empty() || r.starts_with(char::is_whitespace);
    let n = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if (1..=3).contains(&n) && (t[n..].starts_with('.') || t[n..].starts_with(')')) {
        let r = &t[n + 1..];
        return letter(r) || n == 3 && spaced(r);
    }
    let Some(c) = t.chars().next() else { return false };
    if GANADA.contains(c) {
        let r = &t[c.len_utf8()..];
        return (r.starts_with('.') || r.starts_with(')')) && letter(&r[1..]);
    }
    false
}

fn parts(hwpx: &[u8]) -> Option<(String, Vec<(String, String)>)> {
    let mut z = zip::ZipArchive::new(Cursor::new(hwpx)).ok()?;
    let mut read = |name: &str| -> Option<String> {
        let mut s = String::new();
        z.by_name(name).ok()?.read_to_string(&mut s).ok()?;
        Some(s)
    };
    let header = read("Contents/header.xml")?;
    let mut names: Vec<String> = (0..z.len())
        .filter_map(|i| z.by_index(i).ok().map(|f| f.name().to_string()))
        .filter(|n| n.starts_with("Contents/section") && n.ends_with(".xml"))
        .collect();
    names.sort();
    let mut sections = Vec::new();
    for n in names {
        let mut s = String::new();
        z.by_name(&n).ok()?.read_to_string(&mut s).ok()?;
        sections.push((n, s));
    }
    Some((header, sections))
}

/// paraPr id -> (heading type, format)
fn headings(header: &str) -> HashMap<String, (String, Option<String>)> {
    let mut out = HashMap::new();
    let Ok(doc) = roxmltree::Document::parse(header) else { return out };
    let mut numbering: HashMap<String, roxmltree::Node> = HashMap::new();
    let mut bullets: HashMap<String, String> = HashMap::new();
    for n in doc.descendants().filter(|n| n.is_element()) {
        match n.tag_name().name() {
            "numbering" => {
                if let Some(id) = n.attribute("id") {
                    numbering.insert(id.to_string(), n);
                }
            }
            "bullet" => {
                if let Some(id) = n.attribute("id") {
                    bullets.insert(id.to_string(), n.attribute("char").unwrap_or("").to_string());
                }
            }
            _ => {}
        }
    }
    for p in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "paraPr") {
        let (Some(id), Some(h)) = (p.attribute("id"), p.descendants().find(|n| n.is_element() && n.tag_name().name() == "heading")) else { continue };
        let kind = h.attribute("type").unwrap_or("NONE");
        if kind != "NUMBER" && kind != "BULLET" && kind != "OUTLINE" {
            continue;
        }
        let def = h.attribute("idRef").unwrap_or("");
        let format = if kind == "NUMBER" || kind == "OUTLINE" {
            let level: u32 = h.attribute("level").and_then(|l| l.parse().ok()).unwrap_or(0) + 1;
            numbering.get(def).and_then(|n| {
                n.children()
                    .find(|c| c.is_element() && c.tag_name().name() == "paraHead" && c.attribute("level").and_then(|l| l.parse::<u32>().ok()) == Some(level))
                    .and_then(|c| c.text().map(str::to_string))
            })
        } else {
            bullets.get(def).cloned()
        };
        out.insert(id.to_string(), (kind.to_string(), format));
    }
    out
}

fn para_text(p: roxmltree::Node) -> String {
    let mut s = String::new();
    for t in p.descendants().filter(|n| n.is_element() && n.tag_name().name() == "t") {
        // only this paragraph's own runs, not paragraphs nested in a table inside it
        if t.ancestors().find(|a| a.is_element() && a.tag_name().name() == "p") != Some(p) {
            continue;
        }
        for x in t.descendants() {
            if x.is_text() {
                s.push_str(x.text().unwrap_or(""));
            } else if x.is_element() && x.tag_name().name() == "lineBreak" {
                s.push('\n');
            }
        }
    }
    s
}

fn walk(hwpx: &[u8], mut f: impl FnMut(&str, String, Option<&(String, Option<String>)>)) {
    let Some((header, sections)) = parts(hwpx) else { return };
    let heads = headings(&header);
    for (name, xml) in &sections {
        let Ok(doc) = roxmltree::Document::parse(xml) else { continue };
        for p in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "p") {
            let h = p.attribute("paraPrIDRef").and_then(|id| heads.get(id));
            f(name, para_text(p), h);
        }
    }
}

/// Paragraphs Hancom numbers or bullets by itself (with their text as saved).
pub fn auto_paragraphs(hwpx: &[u8]) -> Vec<AutoPara> {
    let mut out = Vec::new();
    walk(hwpx, |part, text, h| {
        if let Some((kind, format)) = h {
            out.push(AutoPara { part: part.to_string(), text, kind: kind.clone(), format: format.clone() });
        }
    });
    out
}

/// Paragraphs whose number would be shown twice.
pub fn doubled_numbering(hwpx: &[u8]) -> Vec<NumberingIssue> {
    let mut out = Vec::new();
    walk(hwpx, |part, text, h| {
        let auto = h.is_some_and(|(k, _)| k == "NUMBER" || k == "OUTLINE");
        let first_line = text.split('\n').next().unwrap_or("");
        // every finding of the paragraph is weighed; a blocking one wins over an ambiguous one
        let mut found: Option<(&'static str, bool)> = None;
        let mut take = |r: Option<(&'static str, bool)>| {
            if r.is_some_and(|(_, b)| b) || found.is_none() {
                found = r.or(found);
            }
        };
        if auto && auto_marker(first_line) {
            take(match date_reading(first_line) {
                // Hancom puts its own number in front: "1. 3. 2. 회의" may be a date or "1. 1. 2. 항목"
                Some(true) => Some(("auto_date", false)),
                Some(false) => Some(("maybe_date", false)),
                None => Some(("auto_and_typed", true)),
            });
        }
        for l in text.split('\n') {
            let Some(n) = typed_marker_len(l) else { continue };
            let rest = &l[n..];
            // "① ② ③" is a row of symbols, not a doubled number
            if typed_marker_len(rest).is_none() || rest.trim_start().starts_with(|c: char| ('①'..='⑳').contains(&c)) {
                continue;
            }
            take(match (date_reading(l), date_reading(rest)) {
                (Some(true), _) | (_, Some(true)) => None, // "3. 2. 회의", "가. 10. 15. 회의"
                (Some(false), _) | (_, Some(false)) => Some(("maybe_date", false)),
                _ => Some(("typed_twice", true)),
            });
        }
        if let Some((reason, blocking)) = found {
            out.push(NumberingIssue { part: part.to_string(), text: text.chars().take(40).collect(), reason, blocking });
        }
    });
    out
}

/// One-line description for error details.
pub fn describe(issues: &[NumberingIssue]) -> String {
    let list: Vec<String> = issues
        .iter()
        .map(|i| format!("\"{}\"({})", i.text, match i.reason { "auto_and_typed" => "자동 번호 문단에 글자 번호", "typed_twice" => "글자 번호 두 번", "auto_date" => "자동 번호 문단이 날짜 모양으로 시작: 날짜(3월 2일 등)인지 번호 중복인지 확인", _ => "같은 숫자 두 번: 번호 중복인지 날짜(1월 1일 등)인지 확인" }))
        .collect();
    format!(
        "{}. 한컴이 자동 번호를 붙이는 문단이면 fmt·값에서 번호를 빼고, 번호가 여러 줄이면 num/ganada 필터 대신 템플릿 번호에 맡긴다",
        list.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::typed_marker_len as m;
    #[test]
    fn markers() {
        assert_eq!(m("1. 당사"), Some(3));
        assert_eq!(m("  12) 항목"), Some(6));
        assert_eq!(m("(3) 항목"), Some(4));
        assert_eq!(m("가. 항목"), Some("가. ".len()));
        assert_eq!(m("(가) 항목"), Some("(가) ".len()));
        assert_eq!(m("①항목"), Some("①".len()));
        assert_eq!(m("2026. 1. 1."), None);
        assert_eq!(m("1.5배"), None);
        assert_eq!(m("가나다"), None);
        assert_eq!(m("다음과 같이"), None);
        assert_eq!(m("100. 개"), None);
        assert_eq!(m(""), None);
    }
    #[test]
    fn dates_are_not_doubled_numbers() {
        use super::{auto_marker, date_reading};
        assert_eq!(date_reading("3. 2. 회의"), Some(true));
        assert_eq!(date_reading("26. 11. 9. 회의"), Some(true));
        assert_eq!(date_reading("1. 1. 당 사와"), Some(false));
        assert_eq!(date_reading("11. 11. 행사"), Some(false));
        assert_eq!(date_reading("13. 32. 항목"), None);
        assert_eq!(date_reading("1) 2) 항목"), None);
        assert_eq!(date_reading("1. 2.5배 증가"), None);
        assert_eq!(date_reading("1. 가. 항목"), None);
        assert!(auto_marker("1.항목") && auto_marker("1)Item") && auto_marker("가.항목") && auto_marker("100. 항목"));
        assert!(!auto_marker("1.5배") && !auto_marker("2026. 1. 1.") && !auto_marker("다음과 같이") && !auto_marker("100.5"));
        assert_eq!(date_reading("12. 12. 착수보고회"), Some(false));
        assert_eq!(date_reading("3. 2.(월) 회의"), Some(true));
    }
}
