//! XLSX for cards. Read (C7): sum one labelled section of a ledger sheet from cached cell
//! values. Write (C8): put text/numbers into existing cells of a template workbook, keeping
//! formulas (their cached value is replaced by the core's number) and asking Excel to
//! recalculate on open. Not a spreadsheet engine.
use crate::card::Extract;
use crate::error::{Error, Result};
use crate::expr::parse_decimal;
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

const MAX_PART: u64 = 32 * 1024 * 1024;
const MAX_TOTAL: u64 = 64 * 1024 * 1024;

fn bad(d: impl Into<String>) -> Error {
    Error::new("invalid_xlsx", d)
}

fn parts(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| bad("zip"))?;
    let mut out = Vec::new();
    let mut total = 0u64;
    for i in 0..z.len() {
        let f = z.by_index(i).map_err(|_| bad("zip entry"))?;
        let name = f.name().to_string();
        let mut b = Vec::new();
        f.take(MAX_PART + 1).read_to_end(&mut b).map_err(|_| bad("zip read"))?;
        total += b.len() as u64;
        if b.len() as u64 > MAX_PART || total > MAX_TOTAL {
            return Err(bad("too large"));
        }
        out.push((name, b));
    }
    Ok(out)
}
fn get<'a>(p: &'a [(String, Vec<u8>)], name: &str) -> Option<&'a str> {
    p.iter().find(|(n, _)| n == name).and_then(|(_, b)| std::str::from_utf8(b).ok())
}
fn doc(x: &str) -> Result<roxmltree::Document<'_>> {
    roxmltree::Document::parse(x).map_err(|_| bad("xml"))
}

/// Sheet name -> part path.
fn sheet_path(p: &[(String, Vec<u8>)], sheet: &str) -> Result<String> {
    let wb = doc(get(p, "xl/workbook.xml").ok_or_else(|| bad("workbook"))?)?;
    let rels = doc(get(p, "xl/_rels/workbook.xml.rels").ok_or_else(|| bad("rels"))?)?;
    let s = wb.descendants().find(|n| n.has_tag_name("sheet") && n.attribute("name") == Some(sheet)).ok_or_else(|| Error::new("xlsx_sheet_missing", sheet))?;
    let rid = s.attributes().find(|a| a.name() == "id").map(|a| a.value()).ok_or_else(|| bad("sheet rid"))?;
    let t = rels.descendants().find(|n| n.has_tag_name("Relationship") && n.attribute("Id") == Some(rid)).and_then(|n| n.attribute("Target")).ok_or_else(|| bad("sheet target"))?;
    let t = t.trim_start_matches('/');
    Ok(if t.starts_with("xl/") { t.to_string() } else { format!("xl/{t}") })
}
fn col_of(addr: &str) -> String {
    addr.chars().take_while(|c| c.is_ascii_alphabetic()).collect()
}
fn row_of(addr: &str) -> Option<u32> {
    addr.chars().skip_while(|c| c.is_ascii_alphabetic()).collect::<String>().parse().ok()
}

/// row -> col -> text (shared/inline strings resolved, numbers as stored, cached formula values).
fn grid(p: &[(String, Vec<u8>)], sheet: &str) -> Result<BTreeMap<u32, BTreeMap<String, String>>> {
    let shared: Vec<String> = match get(p, "xl/sharedStrings.xml") {
        Some(x) => {
            let d = doc(x)?;
            d.descendants()
                .filter(|n| n.has_tag_name("si"))
                .map(|si| si.descendants().filter(|t| t.has_tag_name("t") && !t.ancestors().any(|a| a.has_tag_name("rPh"))).filter_map(|t| t.text()).collect())
                .collect()
        }
        None => vec![],
    };
    let path = sheet_path(p, sheet)?;
    let d = doc(get(p, &path).ok_or_else(|| bad("sheet part"))?)?;
    let mut g: BTreeMap<u32, BTreeMap<String, String>> = BTreeMap::new();
    for c in d.descendants().filter(|n| n.has_tag_name("c")) {
        let Some(r) = c.attribute("r") else { continue };
        let v = c.children().find(|n| n.has_tag_name("v")).and_then(|n| n.text());
        let text = match c.attribute("t") {
            Some("s") => v.and_then(|i| i.parse::<usize>().ok()).and_then(|i| shared.get(i).cloned()),
            Some("inlineStr") => Some(c.descendants().filter(|t| t.has_tag_name("t")).filter_map(|t| t.text()).collect()),
            _ => v.map(String::from),
        };
        if let (Some(t), Some(row)) = (text, row_of(r)) {
            g.entry(row).or_default().insert(col_of(r), t);
        }
    }
    Ok(g)
}

fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// (sum of item values, stated subtotal, item count).
pub fn section_sum(bytes: &[u8], x: &Extract) -> Result<(Decimal, Option<Decimal>, usize)> {
    let p = parts(bytes)?;
    let g = grid(&p, &x.sheet)?;
    let want = squash(&x.section);
    let start = g
        .iter()
        .find(|(_, cols)| cols.get(&x.label_col).is_some_and(|t| squash(t) == want))
        .map(|(r, _)| *r)
        .ok_or_else(|| Error::new("xlsx_section_missing", x.section.clone()))?;
    let (mut sum, mut n, mut stated) = (Decimal::ZERO, 0usize, None);
    for (r, cols) in g.range(start..) {
        if *r > start && cols.get(&x.label_col).is_some_and(|t| !t.trim().is_empty() && squash(t) != want) {
            break; // next section without a subtotal row
        }
        if cols.get(&x.total_col).is_some_and(|t| squash(t) == squash(&x.total_marker)) {
            stated = cols.get(&x.value_col).and_then(|v| parse_decimal(v));
            break;
        }
        if let Some(v) = cols.get(&x.value_col).and_then(|v| parse_decimal(v)) {
            sum += v;
            n += 1;
        }
    }
    Ok((sum, stated, n))
}

pub enum CellValue {
    Text(String),
    Number(Decimal),
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Write values into existing cells of `sheet`; every target cell must exist in the template.
pub fn write_cells(template: &[u8], sheet: &str, cells: &[(String, CellValue)]) -> Result<Vec<u8>> {
    let mut p = parts(template)?;
    let path = sheet_path(&p, sheet)?;
    let xml = get(&p, &path).ok_or_else(|| bad("sheet part"))?.to_string();
    let d = doc(&xml)?;
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    for (addr, val) in cells {
        let c = d
            .descendants()
            .find(|n| n.has_tag_name("c") && n.attribute("r") == Some(addr.as_str()))
            .ok_or_else(|| Error::new("xlsx_cell_missing", addr.clone()))?;
        let style = c.attribute("s").map(|s| format!(" s=\"{s}\"")).unwrap_or_default();
        let formula = c.children().find(|n| n.has_tag_name("f")).map(|f| xml[f.range()].to_string());
        let new = match (val, &formula) {
            (CellValue::Number(n), Some(f)) => format!("<c r=\"{addr}\"{style}>{f}<v>{n}</v></c>"),
            (CellValue::Number(n), None) => format!("<c r=\"{addr}\"{style}><v>{n}</v></c>"),
            (CellValue::Text(_), Some(_)) => return Err(Error::new("xlsx_formula_cell", format!("{addr}: 수식 칸에 글자를 쓰지 않는다"))),
            (CellValue::Text(t), None) => format!("<c r=\"{addr}\"{style} t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>", esc(t)),
        };
        edits.push((c.range(), new));
    }
    edits.sort_by_key(|(r, _)| r.start);
    if edits.windows(2).any(|w| w[0].0.end > w[1].0.start) {
        return Err(Error::new("duplicate_target", "xlsx cell"));
    }
    let mut out = xml.clone();
    for (r, s) in edits.into_iter().rev() {
        out.replace_range(r, &s);
    }
    doc(&out)?;
    // Excel recalculates formulas on open; our cached values are what a non-calculating reader sees.
    let wb = get(&p, "xl/workbook.xml").ok_or_else(|| bad("workbook"))?.to_string();
    let wb = if let Some(i) = wb.find("<calcPr") {
        let end = i + wb[i..].find("/>").ok_or_else(|| bad("calcPr"))?;
        let tag = &wb[i..end];
        if tag.contains("fullCalcOnLoad") { wb.clone() } else { format!("{} fullCalcOnLoad=\"1\"{}", &wb[..end], &wb[end..]) }
    } else {
        let i = wb.find("</workbook>").ok_or_else(|| bad("workbook end"))?;
        format!("{}<calcPr fullCalcOnLoad=\"1\"/>{}", &wb[..i], &wb[i..])
    };
    for (n, b) in p.iter_mut() {
        if *n == path {
            *b = out.clone().into_bytes();
        } else if n == "xl/workbook.xml" {
            *b = wb.clone().into_bytes();
        }
    }
    // calcChain may list cells whose formulas we kept; it stays valid. Rewrite the zip.
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (n, b) in &p {
        w.start_file(n.as_str(), opt).map_err(|_| bad("zip write"))?;
        w.write_all(b).map_err(|_| bad("zip write"))?;
    }
    let bytes = w.finish().map_err(|_| bad("zip write"))?.into_inner();
    // re-read our own output
    let back = parts(&bytes)?;
    sheet_path(&back, sheet)?;
    Ok(bytes)
}
