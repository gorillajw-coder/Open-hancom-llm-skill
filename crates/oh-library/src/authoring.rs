//! Pack authoring checks (LIBRARY-GUIDE 4, 7): the residual scan of anonymized material and the
//! manifest writer. Pure byte/text work; the CLI reads and writes files through oh-host.
//!
//! Scan: every file, and every member of a ZIP container (HWPX, XLSX, a pack zip; nested up to
//! three levels, as dist-scan), is searched in its raw, utf-8, utf-16le and utf-16be views, case-insensitive,
//! for the user's terms (one per line), for e-mail addresses outside example.com, phone-like and
//! resident-number-like patterns, and a preview image that is not blank. The report names terms
//! by line number only, never the terms themselves.
use std::io::Read;

/// Per-member size cap for the scan (the library's own file cap).
pub const SCAN_MEMBER_MAX: u64 = crate::MAX_FILE;
const MAX_DEPTH: usize = 3;

/// One term per line; blank lines and terms shorter than 2 characters are skipped (as dist-scan).
pub fn parse_terms(text: &str) -> Vec<(usize, String)> {
    text.lines().enumerate().map(|(i, l)| (i + 1, l.trim().trim_start_matches('\u{feff}').trim().to_string())).filter(|(_, t)| t.chars().count() >= 2).collect()
}

/// dist-scan rules: an ASCII word term must stand alone ("zorvan" in "zorvantic" is not a hit),
/// a numeric term must not sit inside a longer hex/digit run (image data); other terms match as
/// substrings. `text` and `t` are lowercase.
pub fn term_in(text: &str, t: &str) -> bool {
    if !t.is_ascii() {
        return text.contains(t);
    }
    let digits = t.chars().all(|c| c.is_ascii_digit());
    let blocks = |c: char| if digits { c.is_ascii_hexdigit() } else { c.is_ascii_alphanumeric() };
    text.match_indices(t).any(|(i, _)| {
        let before = text[..i].chars().next_back().is_none_or(|c| !blocks(c));
        let after = text[i + t.len()..].chars().next().is_none_or(|c| !blocks(c));
        before && after
    })
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct MemberHit {
    pub path: String,
    /// line numbers of the terms found
    pub terms: Vec<usize>,
    pub email: usize,
    pub phone: usize,
    pub resident_number: usize,
    /// a `Preview/PrvImage.png` that is not blank (it shows the original page)
    pub preview_image: bool,
    pub hits: usize,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ScanReport {
    pub scanned: usize,
    pub total_hits: usize,
    pub findings: Vec<MemberHit>,
    /// images (other than the preview) the scan cannot read: check them by hand
    pub images_to_check: Vec<String>,
    /// members that could not be read (too large, broken ZIP): treated as findings
    pub unreadable: Vec<String>,
}
impl ScanReport {
    pub fn clean(&self) -> bool {
        self.total_hits == 0 && self.unreadable.is_empty()
    }
}

fn utf16(b: &[u8], le: bool) -> String {
    let u: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| if le { u16::from_le_bytes(*c) } else { u16::from_be_bytes(*c) }).collect();
    String::from_utf16_lossy(&u)
}
fn views(b: &[u8]) -> [String; 3] {
    [String::from_utf8_lossy(b).to_lowercase(), utf16(b, true).to_lowercase(), utf16(b, false).to_lowercase()]
}
fn is_zip(b: &[u8]) -> bool {
    b.starts_with(b"PK\x03\x04") || b.starts_with(b"PK\x05\x06")
}
fn is_image(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".bmp", ".tif", ".tiff", ".wmf", ".emf", ".svg"].iter().any(|e| n.ends_with(e))
}

fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `[\w.+-]+@(?!example\.com)[\w-]+\.[\w.]+`
pub fn emails(text: &str) -> usize {
    let cs: Vec<char> = text.chars().collect();
    let mut n = 0;
    let mut i = 0;
    while i < cs.len() {
        if cs[i] != '@' {
            i += 1;
            continue;
        }
        let local = i > 0 && { let c = cs[i - 1]; word(c) || ".+-".contains(c) };
        let mut j = i + 1;
        while j < cs.len() && (word(cs[j]) || cs[j] == '-') {
            j += 1;
        }
        let label = j > i + 1;
        if local && label && j + 1 < cs.len() && cs[j] == '.' && (word(cs[j + 1]) || cs[j + 1] == '.') {
            let mut k = j + 1;
            while k < cs.len() && (word(cs[k]) || cs[k] == '.') {
                k += 1;
            }
            let domain: String = cs[i + 1..k].iter().collect::<String>().to_lowercase();
            if !domain.starts_with("example.com") {
                n += 1;
            }
            i = k;
        } else {
            i += 1;
        }
    }
    n
}

fn digits_at(cs: &[char], i: usize, lo: usize, hi: usize) -> Vec<usize> {
    let mut run = 0;
    while i + run < cs.len() && cs[i + run].is_ascii_digit() && run < hi {
        run += 1;
    }
    (lo..=run.min(hi)).rev().map(|k| i + k).collect()
}

/// `(?<![\w.])\+?\d{2,4}[ -]\d{2,4}[ -]\d{3,4}(?:[ -]\d{2,4})?(?![\w.])`, `000-0000-0000` excluded.
pub fn phones(text: &str) -> usize {
    let cs: Vec<char> = text.chars().collect();
    let sep = |i: usize| i < cs.len() && (cs[i] == ' ' || cs[i] == '-');
    let free_end = |i: usize| i >= cs.len() || !(word(cs[i]) || cs[i] == '.');
    let mut n = 0;
    let mut i = 0;
    'outer: while i < cs.len() {
        if i > 0 && (word(cs[i - 1]) || cs[i - 1] == '.') {
            i += 1;
            continue;
        }
        let start = if cs[i] == '+' { i + 1 } else { i };
        for a in digits_at(&cs, start, 2, 4) {
            if !sep(a) {
                continue;
            }
            for b in digits_at(&cs, a + 1, 2, 4) {
                if !sep(b) {
                    continue;
                }
                for c in digits_at(&cs, b + 1, 3, 4) {
                    let mut ends = Vec::new();
                    if sep(c) {
                        ends.extend(digits_at(&cs, c + 1, 2, 4));
                    }
                    ends.push(c);
                    for e in ends {
                        if free_end(e) {
                            let m: String = cs[i..e].iter().collect();
                            if m != "000-0000-0000" {
                                n += 1;
                            }
                            i = e;
                            continue 'outer;
                        }
                    }
                }
            }
        }
        i += 1;
    }
    n
}

/// `\d{6}-\d{7}`
pub fn resident_numbers(text: &str) -> usize {
    let cs: Vec<char> = text.chars().collect();
    let d = |i: usize, k: usize| i + k <= cs.len() && cs[i..i + k].iter().all(|c| c.is_ascii_digit());
    let mut n = 0;
    let mut i = 0;
    while i + 14 <= cs.len() {
        if d(i, 6) && cs[i + 6] == '-' && d(i + 7, 7) {
            n += 1;
            i += 14;
        } else {
            i += 1;
        }
    }
    n
}

/// A PNG whose pixels hold at most 3 distinct byte values (filter bytes included), or 1x1.
pub fn png_blank(b: &[u8]) -> Option<bool> {
    if !b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let mut idat = Vec::new();
    let mut i = 8;
    let (mut w, mut h) = (0u32, 0u32);
    while i + 8 <= b.len() {
        let n = u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize;
        let t = &b[i + 4..i + 8];
        let body = b.get(i + 8..i + 8 + n)?;
        if t == b"IHDR" && n >= 8 {
            w = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
            h = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
        }
        if t == b"IDAT" {
            idat.extend_from_slice(body);
        }
        i += 12 + n;
    }
    if w == 1 && h == 1 {
        return Some(true);
    }
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(idat.as_slice()).take(64 * 1024 * 1024).read_to_end(&mut raw).ok()?;
    let mut seen = [false; 256];
    raw.iter().for_each(|x| seen[*x as usize] = true);
    Some(seen.iter().filter(|s| **s).count() <= 3)
}

/// Scan one file's bytes (`path` is the name shown in the report).
pub fn scan_bytes(path: &str, bytes: &[u8], terms: &[(usize, String)], report: &mut ScanReport) {
    scan_at(path, bytes, terms, report, 0)
}

fn scan_at(path: &str, bytes: &[u8], terms: &[(usize, String)], report: &mut ScanReport, depth: usize) {
    report.scanned += 1;
    let mut hit = MemberHit { path: path.to_string(), ..Default::default() };
    let mut v = views(bytes);
    // a regulation table's country list names every country (dist-scan exemption, terms only)
    if path.contains("regulations/") && path.ends_with(".toml") {
        for x in v.iter_mut() {
            *x = x.lines().filter(|l| !l.trim_start().starts_with("places = [")).collect::<Vec<_>>().join("\n");
        }
    }
    for (line, t) in terms {
        let t = t.to_lowercase();
        if v.iter().any(|x| term_in(x, &t)) {
            hit.terms.push(*line);
        }
    }
    // a readable zip container's own bytes are its members' (compressed or stored) data: the
    // patterns are checked on the members, decompressed (dist-scan rule); terms still are here
    let container = is_zip(bytes) && zip::ZipArchive::new(std::io::Cursor::new(bytes)).is_ok();
    if !container {
        let text = format!("{}\n{}", String::from_utf8_lossy(bytes), utf16(bytes, true));
        hit.email = emails(&text);
        hit.phone = phones(&text);
        hit.resident_number = resident_numbers(&text);
    }
    let base = path.rsplit('!').next().unwrap_or(path);
    if base == "Preview/PrvImage.png" {
        hit.preview_image = png_blank(bytes) != Some(true);
    } else if is_image(base) {
        report.images_to_check.push(path.to_string());
    }
    hit.hits = hit.terms.len() + hit.email + hit.phone + hit.resident_number + usize::from(hit.preview_image);
    if hit.hits > 0 {
        report.total_hits += hit.hits;
        report.findings.push(hit);
    }
    if is_zip(bytes) {
        if depth >= MAX_DEPTH {
            report.unreadable.push(format!("{path} (중첩 zip이 너무 깊다)"));
            return;
        }
        let Ok(mut z) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else {
            report.unreadable.push(format!("{path} (zip을 열 수 없다)"));
            return;
        };
        let comment = z.comment().to_vec();
        if !comment.is_empty() {
            scan_at(&format!("{path}!(zip comment)"), &comment, terms, report, depth + 1);
        }
        for i in 0..z.len() {
            let Ok(mut f) = z.by_index(i) else {
                report.unreadable.push(format!("{path}!#{i}"));
                continue;
            };
            let name = f.name().to_string();
            // the member name itself can carry a term
            scan_name(&format!("{path}!{name}"), &name, terms, report);
            if f.is_dir() {
                continue;
            }
            let mut b = Vec::new();
            if (&mut f).take(SCAN_MEMBER_MAX + 1).read_to_end(&mut b).is_err() || b.len() as u64 > SCAN_MEMBER_MAX {
                report.unreadable.push(format!("{path}!{name}"));
                continue;
            }
            scan_at(&format!("{path}!{name}"), &b, terms, report, depth + 1);
        }
    }
}

/// File and member names are checked for terms too (a file named after the client, say).
pub fn scan_name(shown: &str, name: &str, terms: &[(usize, String)], report: &mut ScanReport) {
    let n = name.to_lowercase();
    let lines: Vec<usize> = terms.iter().filter(|(_, t)| term_in(&n, &t.to_lowercase())).map(|(l, _)| *l).collect();
    if !lines.is_empty() {
        report.total_hits += lines.len();
        report.findings.push(MemberHit { path: format!("{shown} (이름)"), hits: lines.len(), terms: lines, ..Default::default() });
    }
}

/// Folders of a pack and the manifest kind of their files (as `scripts/library-manifest.py`).
pub const PACK_DIRS: [(&str, &str); 5] = [("frames", "frame"), ("cards", "card"), ("templates", "template"), ("examples", "example"), ("regulations", "regulation")];

/// The manifest kind of `dir/file`, or None for a file that does not belong there (wrong
/// extension, hidden file): such files are reported, never listed.
pub fn kind_for(dir: &str, file: &str) -> Option<&'static str> {
    let k = PACK_DIRS.iter().find(|(d, _)| *d == dir)?.1;
    let ext = file.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let allowed: &[&str] = match k {
        "frame" | "card" => &["toml"],
        "template" | "example" => &["hwpx", "xlsx"],
        _ => &["toml", "md"],
    };
    if file.starts_with('.') || !allowed.contains(&ext.as_str()) {
        return None;
    }
    Some(if k == "regulation" && file.ends_with(".toml") { "regulation_table" } else { k })
}

fn toml_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04X}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// `manifest.toml` text: (path, sha256, kind) rows in the given order.
pub fn manifest_text(version: &str, anonymized: bool, name: Option<&str>, search: &str, rows: &[(String, String, String)]) -> String {
    let mut out = format!("version = {}\nanonymized = {anonymized}\n", toml_str(version));
    if let Some(n) = name {
        out += &format!("name = {}\n", toml_str(n));
    }
    out += &format!("search = {}\n", toml_str(search));
    for (p, h, k) in rows {
        out += &format!("\n[[file]]\npath = {}\nsha256 = \"{h}\"\nkind = \"{k}\"\n", toml_str(p));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patterns_match_the_python_scanner() {
        assert_eq!(emails(&format!("a kim@example.com b lee{}corp.co.kr c x@y", '@')), 1);
        assert_eq!(emails("홍길동@회사.한국"), 1);
        assert_eq!(phones("tel 02-1234-5678, 000-0000-0000, +82 10 1234 5678"), 2);
        assert_eq!(phones("v1.02-1234-5678"), 0);
        assert_eq!(phones("2026-11-13"), 0);
        assert_eq!(phones("010-1234-56789"), 0);
        assert_eq!(phones("x+82 10 1234 5678"), 1);
        assert_eq!(phones("02 123 4567 89 12"), 1);
        assert_eq!(resident_numbers("900101-1234567 / 000000-00000"), 1);
    }
    #[test]
    fn png_blank_detects_1x1() {
        assert_eq!(png_blank(oh_document_blank()), Some(true));
        assert_eq!(png_blank(b"nope"), None);
    }
    fn oh_document_blank() -> &'static [u8] {
        &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xff, 0xff, 0x3f, 0x00, 0x05, 0xfe, 0x02, 0xfe, 0x0d, 0xef, 0x46,
            0xb8, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ]
    }
    #[test]
    fn terms_are_trimmed_and_numbered() {
        assert_eq!(parse_terms("\u{feff}가나\r\n\r\n 다라 \nx\n"), vec![(1, "가나".to_string()), (3, "다라".to_string())]);
    }
    #[test]
    fn term_rules_follow_dist_scan() {
        assert!(!term_in("zorvantic-approval", "zorvan"));
        assert!(term_in("a zorvan b", "zorvan"));
        assert!(!term_in("ab1234cd", "1234"));
        assert!(term_in("no 1234.", "1234"));
        assert!(term_in("가상사업본부장", "가상사업"));
    }
    #[test]
    fn kinds_need_the_folder_extension() {
        assert_eq!(kind_for("cards", "a.toml"), Some("card"));
        assert_eq!(kind_for("cards", "notes.txt"), None);
        assert_eq!(kind_for("regulations", "x.toml"), Some("regulation_table"));
        assert_eq!(kind_for("regulations", "x.md"), Some("regulation"));
        assert_eq!(kind_for("templates", ".~lock.hwpx"), None);
        assert_eq!(kind_for("templates", "a.HWPX"), Some("template"));
    }
    #[test]
    fn manifest_text_escapes() {
        let t = manifest_text("1.0", true, Some("팀"), "x", &[("cards/a.toml".into(), "ab".into(), "card".into())]);
        assert!(t.starts_with("version = \"1.0\"\nanonymized = true\nname = \"팀\"\n"));
    }
}
