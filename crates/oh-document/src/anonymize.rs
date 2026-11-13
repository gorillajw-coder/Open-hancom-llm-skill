//! Anonymization of an HWPX package for a shared library pack (LIBRARY-GUIDE 4, D6).
//! Fixed string pairs are replaced in the text members (section/header XML, `content.hpf`,
//! `Preview/PrvText.txt`), author metadata is set to a placeholder, the preview image becomes a
//! blank 1x1 PNG, and stale line caches are dropped afterwards. Every other member stays byte
//! identical. The report counts by pair number only; it never repeats the original strings.
use crate::{Package, Prepared, Result};
use std::collections::BTreeMap;

/// 1x1 white PNG (the preview image of the shipped, anonymized templates).
pub const BLANK_PREVIEW_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00,
    0x90, 0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xff, 0xff, 0x3f, 0x00, 0x05, 0xfe, 0x02, 0xfe, 0x0d, 0xef, 0x46, 0xb8, 0x00,
    0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];
/// Author metadata placeholder (D6 인명 → 김철수; the shipped templates carry `kim`).
pub const AUTHOR_PLACEHOLDER: &str = "kim";
const PREVIEW_IMAGE: &str = "Preview/PrvImage.png";
const PREVIEW_TEXT: &str = "Preview/PrvText.txt";
const AUTHOR_META: [&str; 2] = ["creator", "lastsaveby"];

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
/// Start offsets of `pat` in `text` by the residual-scan rules: an ASCII pattern matches without
/// case and only as a whole word (a numeric one not inside a longer hex run); others as written.
fn matches(text: &str, pat: &str) -> Vec<usize> {
    if !pat.is_ascii() {
        return text.match_indices(pat).map(|(i, _)| i).collect();
    }
    let digits = pat.bytes().all(|c| c.is_ascii_digit());
    let blocks = |c: u8| if digits { c.is_ascii_hexdigit() } else { c.is_ascii_alphanumeric() };
    let (tb, pb) = (text.as_bytes(), pat.as_bytes());
    let mut out = Vec::new();
    let mut i = 0;
    while i + pb.len() <= tb.len() {
        if tb[i..i + pb.len()].eq_ignore_ascii_case(pb) && (i == 0 || !blocks(tb[i - 1])) && tb.get(i + pb.len()).is_none_or(|c| !blocks(*c)) {
            out.push(i);
            i += pb.len();
        } else {
            i += 1;
        }
    }
    out
}
fn replace_all(text: &str, pat: &str, to: &str) -> (String, usize) {
    let at = matches(text, pat);
    if at.is_empty() {
        return (text.to_string(), 0);
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for &i in &at {
        out.push_str(&text[last..i]);
        out.push_str(to);
        last = i + pat.len();
    }
    out.push_str(&text[last..]);
    (out, at.len())
}

/// Replace only in character data (between tags), never in tag or attribute names or values.
fn replace_in_text(xml: &str, pat: &str, to: &str) -> (String, usize) {
    let mut out = String::with_capacity(xml.len());
    let mut n = 0;
    let mut rest = xml;
    while !rest.is_empty() {
        match rest.find('<') {
            Some(lt) => {
                let (u, c) = replace_all(&rest[..lt], pat, to);
                out.push_str(&u);
                n += c;
                let gt = rest[lt..].find('>').map(|g| lt + g + 1).unwrap_or(rest.len());
                out.push_str(&rest[lt..gt]);
                rest = &rest[gt..];
            }
            None => {
                let (u, c) = replace_all(rest, pat, to);
                out.push_str(&u);
                n += c;
                rest = "";
            }
        }
    }
    (out, n)
}

fn text_member(name: &str) -> bool {
    name == PREVIEW_TEXT || (name.starts_with("Contents/") && (name.ends_with(".xml") || name.ends_with(".hpf")))
}

/// Replace the text of `<opf:meta name="creator|lastsaveby" ...>TEXT</opf:meta>`.
fn set_author_meta(xml: &str) -> (String, usize) {
    let mut out = xml.to_string();
    let mut n = 0;
    for key in AUTHOR_META {
        let open = format!("<opf:meta name=\"{key}\"");
        let mut from = 0;
        while let Some(i) = out[from..].find(&open).map(|i| i + from) {
            let Some(gt) = out[i..].find('>').map(|g| g + i + 1) else { break };
            if out[..gt].ends_with("/>") {
                from = gt;
                continue;
            }
            let Some(end) = out[gt..].find("</opf:meta>").map(|e| e + gt) else { break };
            if &out[gt..end] != AUTHOR_PLACEHOLDER {
                out.replace_range(gt..end, AUTHOR_PLACEHOLDER);
                n += 1;
            }
            from = gt + AUTHOR_PLACEHOLDER.len();
        }
    }
    (out, n)
}

impl Package {
    /// `pairs`: (original, replacement), applied in order. Returns the new package bytes and a
    /// report: replacements per pair number (1-based), members still holding an original string
    /// (binary members such as images are only reported, never rewritten), images to check by hand.
    pub fn anonymize(&self, pairs: &[(String, String)]) -> Result<Prepared> {
        if pairs.iter().any(|(a, _)| a.is_empty()) {
            return Err("invalid_anonymize_map");
        }
        let mut changed: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut counts = vec![0usize; pairs.len()];
        let mut author = 0;
        let mut preview_blanked = false;
        for (name, bytes) in &self.parts {
            if name == PREVIEW_IMAGE {
                if bytes.as_slice() != BLANK_PREVIEW_PNG {
                    changed.insert(name.clone(), BLANK_PREVIEW_PNG.to_vec());
                    preview_blanked = true;
                }
                continue;
            }
            if !text_member(name) {
                continue;
            }
            let Ok(text) = std::str::from_utf8(bytes) else { return Err("unsupported_xml_encoding") };
            let mut t = text.to_string();
            let xml = name != PREVIEW_TEXT;
            for (k, (a, b)) in pairs.iter().enumerate() {
                let (a, b) = if xml { (xml_escape(a), xml_escape(b)) } else { (a.clone(), b.clone()) };
                let (u, c) = if xml { replace_in_text(&t, &a, &b) } else { replace_all(&t, &a, &b) };
                counts[k] += c;
                t = u;
            }
            if name == "Contents/content.hpf" {
                let (u, n) = set_author_meta(&t);
                author += n;
                t = u;
            }
            if t != text {
                changed.insert(name.clone(), t.into_bytes());
            }
        }
        let replaced = self.prepare_changed(changed, false)?;
        // a changed text length leaves the line cache stale (Hancom 2022 refuses such files)
        let reopened = Package::from_bytes(replaced.bytes.clone())?;
        let (bytes, lineseg_dropped) = match reopened.drop_stale_linesegs()? {
            Some(p) => (p.bytes, true),
            None => (replaced.bytes, false),
        };
        let out = Package::from_bytes(bytes.clone())?;
        // anything an original string still sits in (any member, utf-8 or utf-16 view)
        let mut remaining: Vec<serde_json::Value> = Vec::new();
        for (name, b) in &out.parts {
            let views = [String::from_utf8_lossy(b).into_owned(), utf16(b, true), utf16(b, false)];
            let left: Vec<usize> = pairs.iter().enumerate().filter(|(_, (a, _))| views.iter().any(|v| !matches(v, a).is_empty() || !matches(v, &xml_escape(a)).is_empty())).map(|(k, _)| k + 1).collect();
            if !left.is_empty() {
                remaining.push(serde_json::json!({"member": name, "pairs": left}));
            }
        }
        let images: Vec<&String> = out.parts.iter().map(|(n, _)| n).filter(|n| n.starts_with("BinData/")).collect();
        let receipt = serde_json::json!({
            "sha256": replaced_sha(&bytes),
            "replaced": counts.iter().enumerate().map(|(k, c)| serde_json::json!({"pair": k + 1, "count": c})).collect::<Vec<_>>(),
            "author_meta_set": author,
            "preview_image_blanked": preview_blanked,
            "line_cache_dropped": lineseg_dropped,
            "remaining": remaining,
            "images_to_check": images,
            "note": "글자가 여러 조각(run)으로 나뉜 원문은 바뀌지 않을 수 있다. 결과를 oh library scan으로 다시 확인한다. 내장 이미지는 손으로 확인한다",
        });
        Ok(Prepared { bytes, receipt })
    }
}

fn utf16(b: &[u8], le: bool) -> String {
    let u: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| if le { u16::from_le_bytes(*c) } else { u16::from_be_bytes(*c) }).collect();
    String::from_utf16_lossy(&u)
}
fn replaced_sha(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn author_meta_is_replaced_once_per_tag() {
        let x = r#"<a><opf:meta name="creator" content="text">홍길동</opf:meta><opf:meta name="lastsaveby" content="text">kim</opf:meta><opf:meta name="date" content="text">x</opf:meta></a>"#;
        let (o, n) = set_author_meta(x);
        assert_eq!(n, 1);
        assert!(o.contains(r#"name="creator" content="text">kim<"#) && o.contains(">x<"));
    }
    #[test]
    fn ascii_pairs_match_whole_words_without_case() {
        assert_eq!(replace_all("Zorvan ZORVAN zorvania", "zorvan", "X"), ("X X zorvania".to_string(), 2));
        assert_eq!(replace_all("본부장 가상사업본부", "가상사업본부", "예시본부"), ("본부장 예시본부".to_string(), 1));
        assert_eq!(replace_all("ab1234 1234", "1234", "0000"), ("ab1234 0000".to_string(), 1));
    }
    #[test]
    fn xml_tags_and_attributes_are_never_touched() {
        let x = r#"<hp:run charPrIDRef="1"><hp:t>run Run 회사</hp:t></hp:run><a name="회사">회사</a>"#;
        let (o, n) = replace_in_text(x, "run", "X");
        assert_eq!((o.as_str(), n), (r#"<hp:run charPrIDRef="1"><hp:t>X X 회사</hp:t></hp:run><a name="회사">회사</a>"#, 2));
        let (o, n) = replace_in_text(x, "회사", "개좋소");
        assert_eq!(n, 2);
        assert!(o.contains(r#"name="회사""#) && o.contains("<hp:t>run Run 개좋소</hp:t>"));
    }
    #[test]
    fn xml_escape_keeps_plain_korean() {
        assert_eq!(xml_escape("A&B <본부>"), "A&amp;B &lt;본부&gt;");
    }
}
