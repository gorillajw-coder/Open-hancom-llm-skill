//! Native character definitions, lexically patched without serializing opaque XML.
use crate::{Package, Result, HP, tag_end, escape_xml};
use serde::Deserialize;
use std::{collections::BTreeMap, ops::Range};
use roxmltree::Node;
pub(crate) const HH: &str = "http://www.hancom.co.kr/hwpml/2011/head";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormatSpec {
    revision: String,
    target: String,
    properties: CharacterProperties,
    range: Option<TextRange>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextRange { start: usize, end: usize }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CharacterProperties {
    height: Option<u32>,
    bold: Option<bool>,
    italic: Option<bool>,
    font_refs: Option<FontRefs>,
    #[serde(rename="textColor")]
    text_color: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FontRefs {hangul:Option<u32>,latin:Option<u32>,hanja:Option<u32>,japanese:Option<u32>,other:Option<u32>,symbol:Option<u32>,user:Option<u32>}

pub(crate) fn patch(xml: &str, mut edits: Vec<(Range<usize>, String)>) -> Result<String> {
    edits.sort_by_key(|(r,_)| r.start);
    if edits.windows(2).any(|p| p[0].0.end > p[1].0.start) { return Err("overlapping_targets"); }
    let mut result = xml.to_string();
    for (r,s) in edits.into_iter().rev() { result.replace_range(r, &s); }
    Ok(result)
}
pub(crate) fn set_attr(xml: &str, node: Node, name: &str, value: &str) -> Result<String> {
    let range = node.range();
    let mut raw = xml[range.clone()].to_string();
    let escaped = escape_xml(value).replace('"', "&quot;").replace('\n', "&#10;").replace('\t', "&#9;");
    if let Some(a) = node.attributes().find(|a| a.name() == name && a.namespace().is_none()) {
        let r = a.range_value();
        // Replace the entire attribute (including quotes) so single-quoted inputs are safe.
        let ar = a.range();
        let _ = r;
        raw.replace_range(ar.start-range.start..ar.end-range.start, &format!("{name}=\"{escaped}\""));
    } else {
        let end = tag_end(&raw).ok_or("invalid_xml")?;
        let at = if raw[..end].ends_with("/>") { end-2 } else { end-1 };
        raw.insert_str(at, &format!(" {name}=\"{escaped}\""));
    }
    Ok(raw)
}
pub(crate) fn attr(raw: &str, name: &str, value: &str) -> Result<String> {
    // Candidate carries all namespace declarations from the original header root.
    let doc = roxmltree::Document::parse(raw).map_err(|_| "invalid_header_xml")?;
    set_attr(raw, doc.root_element(), name, value)
}
pub(crate) fn signature(n: Node) -> String {
    fn visit(n: Node, out: &mut String) {
        if n.is_element() {
            out.push_str(&format!("E{:?}{:?}", n.tag_name().namespace(), n.tag_name().name()));
            let mut attrs: Vec<_> = n.attributes().filter(|a| !(n.parent().is_some_and(|p| p.is_root()) && a.name()=="id" && a.namespace().is_none()))
                .map(|a| (a.namespace(), a.name(), a.value())).collect();
            attrs.sort();
            out.push_str(&format!("{:?}", attrs));
            for c in n.children() { visit(c, out); }
            out.push_str("/E");
        } else if n.is_text() {
            out.push_str(&format!("T{:?}", n.text()));
        } else if n.is_comment() { out.push_str(&format!("C{:?}", n.text())); }
        else if let Some(pi)=n.pi() {out.push_str(&format!("P{:?}{:?}",pi.target,pi.value));}
    }
    let mut result = String::new();
    visit(n, &mut result);
    result
}
pub(crate) fn standalone(xml: &str, node: Node) -> Result<String> {
    let raw = &xml[node.range()];
    let at = raw.find([' ', '\t', '\r', '\n', '/', '>']).ok_or("invalid_xml")?;
    let mut out = raw.to_string();
    let opening=&raw[..tag_end(raw).ok_or("invalid_xml")?];
    let bytes=opening.as_bytes();
    let mut declared=Vec::new();
    let mut pos=at;
    while pos<bytes.len() {
        while pos<bytes.len() && bytes[pos].is_ascii_whitespace() {pos+=1;}
        if pos>=bytes.len() || bytes[pos]==b'/' || bytes[pos]==b'>' {break;}
        let begin=pos;
        while pos<bytes.len() && !bytes[pos].is_ascii_whitespace() && bytes[pos]!=b'=' {pos+=1;}
        let name=&opening[begin..pos];
        while pos<bytes.len() && (bytes[pos].is_ascii_whitespace() || bytes[pos]==b'=') {pos+=1;}
        if pos>=bytes.len() || ![b'\'',b'"'].contains(&bytes[pos]) {return Err("invalid_xml");}
        let quote=bytes[pos];pos+=1;
        while pos<bytes.len() && bytes[pos]!=quote {pos+=1;}
        pos+=1;
        if name=="xmlns" || name.starts_with("xmlns:") {declared.push(name.to_string());}
    }
    let ns = node.namespaces().filter(|n| !declared.contains(&n.name().map(|p| format!("xmlns:{p}")).unwrap_or("xmlns".into()))).map(|n| match n.name() {
        Some(p) => format!(" xmlns:{p}=\"{}\"", escape_xml(n.uri()).replace('"',"&quot;")),
        None => format!(" xmlns=\"{}\"", escape_xml(n.uri()).replace('"',"&quot;")),
    }).collect::<String>();
    // A self-contained copied definition already has these declarations.
    out.insert_str(at, &ns);
    Ok(out)
}

impl Package {
    pub fn format_bytes(&self,spec:FormatSpec)->Result<crate::Prepared> {
        if spec.revision != self.inspection.revision { return Err("stale_document"); }
        let p = spec.properties;
        if p.height.is_none() && p.bold.is_none() && p.italic.is_none() && p.font_refs.is_none() && p.text_color.is_none() { return Err("unsupported_character_properties"); }
        if p.height.is_some_and(|h| !(1..=100000).contains(&h)) { return Err("invalid_character_height"); }
        if p.text_color.as_ref().is_some_and(|s| s.len()!=7 || !s.starts_with('#') || !s[1..].bytes().all(|b| b.is_ascii_hexdigit())) {
            return Err("invalid_text_color");
        }
        let n = self.inspection.text_nodes.iter().find(|n| n.id == spec.target).ok_or("unknown_target")?;
        let section = std::str::from_utf8(&self.parts.iter().find(|(k,_)| k==&n.part).ok_or("unknown_target")?.1).map_err(|_| "unsupported_xml_encoding")?;
        let doc = roxmltree::Document::parse(section).map_err(|_| "invalid_section_xml")?;
        let text = doc.descendants().find(|x| x.is_element() && x.range()==n.range).ok_or("unknown_target")?;
        let run = text.parent().filter(|x| x.has_tag_name((HP,"run"))).ok_or("mixed_run_requires_split")?;
        if !n.editable || run.children().any(|c| !(c.has_tag_name((HP,"t")) && c.children().all(|c| c.is_text())) && !(c.is_text() && c.text().unwrap_or("").trim().is_empty())) {
            return Err("mixed_run_requires_split");
        }
        let chars: Vec<_> = n.text.chars().collect();
        let (start,end) = spec.range.map(|r| (r.start,r.end)).unwrap_or((0,chars.len()));
        if start>end || end>chars.len() || (start==end && !chars.is_empty()) { return Err("invalid_text_range"); }
        let header = std::str::from_utf8(&self.parts.iter().find(|(k,_)| k=="Contents/header.xml").ok_or("missing_header")?.1).map_err(|_| "unsupported_xml_encoding")?;
        let hdoc = roxmltree::Document::parse(header).map_err(|_| "invalid_header_xml")?;
        let sets: Vec<_> = hdoc.descendants().filter(|x| x.has_tag_name((HH,"charProperties"))).collect();
        if sets.len()!=1 { return Err("missing_character_definitions"); }
        let set = sets[0];
        let defs: Vec<_> = set.children().filter(|n| n.is_element()).collect();
        if set.attribute("itemCnt")!=Some(defs.len().to_string().as_str()) || defs.iter().enumerate().any(|(i,d)| !d.has_tag_name((HH,"charPr")) || d.attribute("id")!=Some(i.to_string().as_str())) {
            return Err("nonordinal_character_definitions");
        }
        let old_id = run.attribute("charPrIDRef").ok_or("unresolved_character_reference")?;
        let original = defs.iter().find(|d| d.attribute("id")==Some(old_id)).ok_or("unresolved_character_reference")?;
        let mut candidate = standalone(header, *original)?;
        if let Some(refs)=p.font_refs {
            let entries=[("hangul","HANGUL",refs.hangul),("latin","LATIN",refs.latin),("hanja","HANJA",refs.hanja),("japanese","JAPANESE",refs.japanese),("other","OTHER",refs.other),("symbol","SYMBOL",refs.symbol),("user","USER",refs.user)];
            if entries.iter().all(|(_,_,id)|id.is_none()) {return Err("empty_font_refs");}
            for (key,lang,id) in entries {if let Some(id)=id {
                let groups:Vec<_>=hdoc.descendants().filter(|n|n.has_tag_name((HH,"fontface")) && n.attribute("lang")==Some(lang)).collect();
                let id=id.to_string();
                if groups.len()!=1 || !groups[0].children().any(|n|n.has_tag_name((HH,"font")) && n.attribute("id")==Some(id.as_str())) {return Err("unresolved_font_reference");}
                let cd=roxmltree::Document::parse(&candidate).map_err(|_|"invalid_header_xml")?;
                let nodes:Vec<_>=cd.root_element().children().filter(|n|n.has_tag_name((HH,"fontRef"))).collect();
                if nodes.len()!=1 {return Err("unsupported_font_reference_structure");}
                candidate=patch(&candidate,vec![(nodes[0].range(),set_attr(&candidate,nodes[0],key,&id)?)])?;
            }}
        }
        if let Some(h) = p.height { candidate=attr(&candidate,"height",&h.to_string())?; }
        if let Some(c) = p.text_color { candidate=attr(&candidate,"textColor",&c.to_ascii_uppercase())?; }
        for (tag,flag) in [("italic",p.italic),("bold",p.bold)] {if let Some(enabled) = flag {
            let cd = roxmltree::Document::parse(&candidate).map_err(|_| "invalid_header_xml")?;
            let root = cd.root_element();
            let existing: Vec<_> = root.children().filter(|n| n.has_tag_name((HH,tag))).collect();
            let mut edits: Vec<_> = existing.iter().map(|n| (n.range(), String::new())).collect();
            if enabled {
                let prefix=root.lookup_prefix(HH).map(|p| format!("{p}:")).unwrap_or_default();
                let later=["bold","underline","strikeout","outline","shadow","emboss","engrave","supscript","subscript"];
                let at=root.children().find(|n| n.is_element() && n.tag_name().namespace()==Some(HH) && later.contains(&n.tag_name().name()) && (tag=="italic" || n.tag_name().name()!="bold"))
                    .map(|n| n.range().start).unwrap_or(candidate.rfind("</").ok_or("unsupported_empty_character_definition")?);
                edits.push((at..at,format!("<{prefix}{tag}/>")));
            }
            candidate=patch(&candidate,edits)?;
        }}
        // Compare namespace-aware structures, ignoring only the root definition id.
        let cd = roxmltree::Document::parse(&candidate).map_err(|_| "invalid_header_xml")?;
        let desired=signature(cd.root_element());
        let chosen=defs.iter().find_map(|d| {
            let s=standalone(header,*d).ok()?;
            let doc=roxmltree::Document::parse(&s).ok()?;
            (signature(doc.root_element())==desired).then(|| d.attribute("id").unwrap().to_string())
        });
        let mut changed=BTreeMap::new();
        let id=if let Some(id)=chosen { id } else {
            if defs.len()>=10000 { return Err("excess_style_definitions"); }
            let id=defs.len().to_string();
            candidate=attr(&candidate,"id",&id)?;
            let set_raw=set_attr(header,set,"itemCnt",&(defs.len()+1).to_string())?;
            let at=set_raw.rfind("</").ok_or("invalid_header_xml")?;
            let mut new_set=set_raw;
            new_set.insert_str(at,&candidate);
            changed.insert("Contents/header.xml".to_string(),patch(header,vec![(set.range(),new_set)])?.into_bytes());
            id
        };
        if id!=old_id {
            let replacement=if start==0 && end==chars.len() && run.children().filter(|c| c.is_element()).count()==1 {
                set_attr(section,run,"charPrIDRef",&id)?
            } else {
                if run.attributes().any(|a| a.name()=="id") || text.attributes().any(|a| a.name()=="id") { return Err("cannot_duplicate_run_or_text_id"); }
                let raw=&section[run.range()];
                let open=&raw[..tag_end(raw).ok_or("invalid_section_xml")?];
                let close=&raw[raw.rfind("</").ok_or("invalid_section_xml")?..];
                let styled=set_attr(section,run,"charPrIDRef",&id)?;
                let styled_open=&styled[..tag_end(&styled).ok_or("invalid_section_xml")?];
                let mut result=String::new();
                for child in run.children() {
                    if child.is_text() { result.push_str(&section[child.range()]); continue; }
                    let c_raw=&section[child.range()];
                    if child!=text { result.push_str(&format!("{open}{c_raw}{close}")); continue; }
                    let t_open=&c_raw[..tag_end(c_raw).ok_or("invalid_section_xml")?];
                    let t_close=&c_raw[c_raw.rfind("</").ok_or("unsupported_empty_range_split")?..];
                    for (a,b,opening) in [(0,start,open),(start,end,styled_open),(end,chars.len(),open)] {
                        if a==b { continue; }
                        let value=escape_xml(&chars[a..b].iter().collect::<String>());
                        result.push_str(&format!("{opening}{t_open}{value}{t_close}{close}"));
                    }
                }
                result
            };
            changed.insert(n.part.clone(),patch(section,vec![(run.range(),replacement)])?.into_bytes());
        }
        for data in changed.values() {
            roxmltree::Document::parse(std::str::from_utf8(data).map_err(|_| "invalid_xml")?).map_err(|_| "invalid_xml")?;
        }
        let mut result=self.prepare_changed(changed,false)?;
        result.receipt["character_definition"]=serde_json::json!(id);
        Ok(result)
    }
}
