//! Conservative package/reference checks grounded in the pinned Hancom public model.
//! No XSD engine, native renderer or full-conformance claim.
use crate::{Package,HP,is_section};
use crate::styles::HH;
use serde_json::{Value,json};
use std::collections::{BTreeMap,BTreeSet};
pub(crate) const OPF:&str="http://www.idpf.org/2007/opf/";
pub(crate) const HC:&str="http://www.hancom.co.kr/hwpml/2011/core";
pub(crate) fn safe_part(s:&str)->bool {
    !s.is_empty() && !s.starts_with('/') && !s.contains(['\\',':','%','?','#']) &&
    !s.chars().any(|c|c.is_control()) && s.split('/').all(|p| !p.is_empty() && p!="." && p!="..")
}
impl Package {
    pub(crate) fn model_diagnostics(&self)->Vec<Value> {
        let mut out=Vec::new();
        let mut error=|code:&str,part:&str|out.push(json!({"severity":"error","code":code,"part":part}));
        let parts:BTreeMap<_,_>=self.parts.iter().map(|(n,b)|(n.as_str(),b)).collect();
        let parse=|name:&str| parts.get(name).and_then(|b| std::str::from_utf8(b).ok()).and_then(|s|roxmltree::Document::parse(s).ok());
        let mut items=BTreeMap::new();
        if let Some(doc)=parse("Contents/content.hpf") {
            let root=doc.root_element();
            if !root.has_tag_name((OPF,"package")) {error("invalid_package_root","Contents/content.hpf");}
            let manifests:Vec<_>=root.children().filter(|n|n.has_tag_name((OPF,"manifest"))).collect();
            let spines:Vec<_>=root.children().filter(|n|n.has_tag_name((OPF,"spine"))).collect();
            if manifests.len()!=1 || spines.len()!=1 {error("missing_or_duplicate_manifest_spine","Contents/content.hpf");}
            if let Some(m)=manifests.first() {for n in m.children().filter(|n|n.is_element()) {
                let id=n.attribute("id").unwrap_or("");let href=n.attribute("href").unwrap_or("");
                if !n.has_tag_name((OPF,"item")) || id.is_empty() || n.attribute("media-type").is_none() {error("invalid_manifest_item","Contents/content.hpf");}
                if items.insert(id.to_string(),(href.to_string(),n.attribute("media-type").unwrap_or("").to_string())).is_some() {error("duplicate_manifest_id","Contents/content.hpf");}
                if !safe_part(href) || n.attribute("isEmbeded")==Some("0") {error("unsafe_manifest_href","Contents/content.hpf");}
                else if !parts.contains_key(href) {error("missing_manifest_target","Contents/content.hpf");}
                if n.attribute("media-type")==Some("image/jpeg") && (n.attribute("isEmbeded")!=Some("1") || !href.starts_with("BinData/") || !(href.ends_with(".jpg") || href.ends_with(".jpeg"))) {error("unsupported_jpeg_manifest","Contents/content.hpf");}
            }}
            let mut spine=BTreeSet::new();
            if let Some(s)=spines.first() {for n in s.children().filter(|n|n.is_element()) {
                let id=n.attribute("idref").unwrap_or("");
                if !n.has_tag_name((OPF,"itemref")) || !items.contains_key(id) {error("unresolved_spine_reference","Contents/content.hpf");}
                if !spine.insert(id) {error("duplicate_spine_reference","Contents/content.hpf");}
            }}
            for (name,_) in &self.parts {if (is_section(name) || name=="Contents/header.xml") && !items.iter().any(|(id,(href,_))|href==name && spine.contains(id.as_str())) {error("missing_spine_part",name);}}
        }
        if let Some(doc)=parse("META-INF/container.xml") {
            let roots:Vec<_>=doc.descendants().filter(|n|n.has_tag_name(("urn:oasis:names:tc:opendocument:xmlns:container","rootfile"))).collect();
            if !roots.iter().any(|n|n.attribute("full-path")==Some("Contents/content.hpf")) || roots.iter().any(|n|!n.attribute("full-path").is_some_and(|p|safe_part(p) && parts.contains_key(p))) {error("invalid_container_reference","META-INF/container.xml");}
        }
        if let Some(doc)=parse("Contents/header.xml") {
            if doc.root_element().attribute("secCnt").and_then(|s|s.parse::<usize>().ok())!=Some(self.inspection.section_count) {error("section_count_mismatch","Contents/header.xml");}
            let mut fonts=BTreeMap::new();
            for face in doc.descendants().filter(|n|n.has_tag_name((HH,"fontface"))) {
                let ids:BTreeSet<_>=face.children().filter(|n|n.has_tag_name((HH,"font"))).filter_map(|n|n.attribute("id")).collect();
                if face.attribute("fontCnt").and_then(|s|s.parse::<usize>().ok())!=Some(ids.len()) {error("invalid_font_count","Contents/header.xml");}
                if fonts.insert(face.attribute("lang").unwrap_or(""),ids).is_some() {error("duplicate_font_language","Contents/header.xml");}
            }
            for n in doc.descendants().filter(|n|n.has_tag_name((HH,"fontRef"))) {for (lang,attr) in [("HANGUL","hangul"),("LATIN","latin"),("HANJA","hanja"),("JAPANESE","japanese"),("OTHER","other"),("SYMBOL","symbol"),("USER","user")] {
                if !n.attribute(attr).is_some_and(|id| fonts.get(lang).is_some_and(|ids|ids.contains(id))) {error("unresolved_font_reference","Contents/header.xml");}
            }}
        }
        for (href,mime) in items.values() {if mime.starts_with("image/") {
            let result=parts.get(href.as_str()).ok_or("missing_media_resource").and_then(|b|match mime.as_str() {"image/png"=>crate::inline::png_info(b),"image/jpeg"=>crate::jpeg::info(b),_=>Err("unsupported_image_type")});
            if let Err(code)=result {error("invalid_or_unsupported_image_media",href);if mime=="image/jpeg" {error(code,href);}}
        }}
        for (name,_) in &self.parts {if is_section(name) {if let Some(doc)=parse(name) {
            for pic in doc.descendants().filter(|n|n.has_tag_name((HP,"pic"))) {
                let jpeg=pic.children().filter(|n|n.has_tag_name((HC,"img"))).any(|n|n.attribute("binaryItemIDRef").is_some_and(|id|items.get(id).is_some_and(|(_,m)|m=="image/jpeg")));
                if jpeg && crate::media_edit::supported_picture(doc.input_text(),pic).is_err() {error("unsupported_jpeg_picture_structure",name);}
            }
            for n in doc.descendants().filter(|n|n.has_tag_name((HC,"img"))) {
                if !n.attribute("binaryItemIDRef").is_some_and(|id|items.get(id).is_some_and(|(href,mime)|href.starts_with("BinData/") && parts.contains_key(href.as_str()) && mime.starts_with("image/"))) {error("unresolved_image_reference",name);}
            }
            let mut fields=BTreeSet::new();let mut stack=Vec::new();
            for n in doc.descendants().filter(|n|n.is_element()) {
                let required:&[&str]=if n.has_tag_name((HP,"p")) {&["id","paraPrIDRef","styleIDRef","pageBreak","columnBreak","merged"]}
                    else if n.has_tag_name((HP,"run")) {&["charPrIDRef"]}
                    else if n.has_tag_name((HP,"pagePr")) {&["landscape","width","height","gutterType"]}
                    else {&[]};
                if required.iter().any(|key|n.attribute(*key).is_none()) {error("missing_required_model_attribute",name);}
                if n.has_tag_name((HP,"fieldBegin")) && n.attribute("type")==Some("HYPERLINK") {
                    let params:Vec<_>=n.children().filter(|n|n.has_tag_name((HP,"parameters"))).collect();
                    let strings:Vec<_>=n.descendants().filter(|n|n.has_tag_name((HP,"stringParam"))).collect();
                    let paths:Vec<_>=strings.iter().filter(|n|n.attribute("name")==Some("Path")).collect();
                    let commands:Vec<_>=strings.iter().filter(|n|n.attribute("name")==Some("Command")).collect();
                    if params.len()!=1 || params[0].attribute("cnt").and_then(|s|s.parse::<usize>().ok())!=Some(params[0].children().filter(|n|n.is_element()).count()) {error("invalid_hyperlink_parameters",name);}
                    if paths.len()!=1 || !paths[0].text().is_some_and(crate::inline::url_ok) {error("unsafe_hyperlink",name);}
                    else {
                        let url=paths[0].text().unwrap_or("");let mut expected=String::new();
                        for c in url.chars() {if [':','?',';','#'].contains(&c) {expected.push('\\');}expected.push(c);}expected.push_str(";1;0;0;");
                        if commands.len()!=1 || commands[0].text()!=Some(expected.as_str()) {error("hyperlink_command_mismatch",name);}
                    }
                }
                if n.has_tag_name((HP,"fieldBegin")) {let id=n.attribute("id").unwrap_or("");if id.is_empty() || !fields.insert(id) {error("invalid_field_id",name);}stack.push((id,n.attribute("fieldid")));}
                if n.has_tag_name((HP,"fieldEnd")) && stack.pop()!=Some((n.attribute("beginIDRef").unwrap_or(""),n.attribute("fieldid"))) {error("unpaired_field_control",name);}
            }
            if !stack.is_empty() {error("unpaired_field_control",name);}
        }}}
        out
    }
}
