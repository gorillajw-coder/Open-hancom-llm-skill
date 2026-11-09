//! Minimal XLSX reader: `시트!주소: 값` lines for non-empty cells (cached formula values,
//! shared/inline strings, numbers as stored). Not a spreadsheet engine; dates stay serials.
use std::io::Read;
use zip::ZipArchive;

const MAX_PART: u64 = 20 * 1024 * 1024;

fn part<R: Read + std::io::Seek>(z: &mut ZipArchive<R>, name: &str) -> Result<Option<String>, &'static str> {
    let Ok(f) = z.by_name(name) else { return Ok(None) };
    let mut s = String::new();
    f.take(MAX_PART + 1).read_to_string(&mut s).map_err(|_| "invalid_xlsx")?;
    if s.len() as u64 > MAX_PART {
        return Err("invalid_xlsx");
    }
    Ok(Some(s))
}

fn text_of(n: roxmltree::Node) -> String {
    n.descendants()
        .filter(|d| d.has_tag_name("t") && !d.ancestors().any(|a| a.has_tag_name("rPh")))
        .filter_map(|d| d.text())
        .collect()
}

/// Returns (lines, truncated). Output stops once `max_chars` characters are collected.
pub fn extract(bytes: &[u8], max_chars: usize) -> Result<(Vec<String>, bool), &'static str> {
    let mut z = ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| "invalid_xlsx")?;
    let wb = part(&mut z, "xl/workbook.xml")?.ok_or("invalid_xlsx")?;
    let rels = part(&mut z, "xl/_rels/workbook.xml.rels")?.ok_or("invalid_xlsx")?;
    let shared = match part(&mut z, "xl/sharedStrings.xml")? {
        Some(x) => {
            let d = roxmltree::Document::parse(&x).map_err(|_| "invalid_xlsx")?;
            d.descendants().filter(|n| n.has_tag_name("si")).map(text_of).collect::<Vec<_>>()
        }
        None => vec![],
    };
    let wbd = roxmltree::Document::parse(&wb).map_err(|_| "invalid_xlsx")?;
    let rd = roxmltree::Document::parse(&rels).map_err(|_| "invalid_xlsx")?;
    let mut lines = Vec::new();
    let (mut used, mut truncated) = (0usize, false);
    'sheets: for s in wbd.descendants().filter(|n| n.has_tag_name("sheet")) {
        let name = s.attribute("name").unwrap_or("?");
        let rid = s.attributes().find(|a| a.name() == "id").map(|a| a.value()).ok_or("invalid_xlsx")?;
        let target = rd
            .descendants()
            .find(|n| n.has_tag_name("Relationship") && n.attribute("Id") == Some(rid))
            .and_then(|n| n.attribute("Target"))
            .ok_or("invalid_xlsx")?;
        let path = match target.strip_prefix('/') {
            Some(abs) => abs.to_string(),
            None => format!("xl/{target}"),
        };
        if path.contains("..") {
            return Err("invalid_xlsx");
        }
        let Some(xml) = part(&mut z, &path)? else { continue };
        let d = roxmltree::Document::parse(&xml).map_err(|_| "invalid_xlsx")?;
        for c in d.descendants().filter(|n| n.has_tag_name("c")) {
            let addr = c.attribute("r").unwrap_or("?");
            let t = c.attribute("t").unwrap_or("n");
            let v = c.children().find(|n| n.has_tag_name("v")).and_then(|n| n.text());
            let value = match t {
                "s" => v.and_then(|i| i.trim().parse::<usize>().ok()).and_then(|i| shared.get(i).cloned()),
                "inlineStr" => c.children().find(|n| n.has_tag_name("is")).map(text_of),
                _ => v.map(str::to_string),
            };
            let Some(value) = value.filter(|v| !v.trim().is_empty()) else { continue };
            let line = format!("{name}!{addr}: {value}");
            used += line.chars().count() + 1;
            if used > max_chars {
                truncated = true;
                break 'sheets;
            }
            lines.push(line);
        }
    }
    Ok((lines, truncated))
}
