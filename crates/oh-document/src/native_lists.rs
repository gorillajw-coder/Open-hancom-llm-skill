//! Saved-XML bounded native list bindings. No counter simulation or rendering claims.
use crate::styles::HH;
use crate::ObjectNode;
use serde_json::{Value,json};

fn child<'a,'input>(node:roxmltree::Node<'a,'input>,tag:&str)->Option<roxmltree::Node<'a,'input>> {
    let mut found=node.children().filter(|n|n.has_tag_name((HH,tag)));
    let n=found.next()?;if found.next().is_some(){None}else{Some(n)}
}
fn definition<'a,'input>(root:roxmltree::Node<'a,'input>,group:&str,tag:&str,id:&str)->Option<roxmltree::Node<'a,'input>> {
    let registry=child(child(root,"refList")?,group)?;
    let mut found=registry.children().filter(|n|n.has_tag_name((HH,tag)) && n.attribute("id")==Some(id));
    let n=found.next()?;if found.next().is_some(){None}else{Some(n)}
}
fn exact_attributes(node:roxmltree::Node<'_, '_>,expected:&[(&str,&str)])->bool {
    node.attributes().len()==expected.len() && expected.iter().all(|(key,value)|node.attribute(*key)==Some(*value)) && node.attributes().all(|a|a.namespace().is_none())
}
fn attributes(node:roxmltree::Node<'_, '_>)->Value {
    json!(node.attributes().map(|a|(a.name(),a.value())).collect::<std::collections::BTreeMap<_,_>>())
}
fn marker_binding(root:roxmltree::Node<'_, '_>,cid:&str)->Option<Value> {
    let c=definition(root,"charProperties","charPr",cid)?;
    let refs=child(c,"fontRef")?;
    let registry=child(child(root,"refList")?,"fontfaces")?;
    let mut fonts=serde_json::Map::new();
    for script in ["hangul","latin","hanja","japanese","other","symbol","user"] {
        let id=refs.attribute(script)?;
        let lang=script.to_ascii_uppercase();
        let mut faces=registry.children().filter(|n|n.has_tag_name((HH,"fontface")) && n.attribute("lang")==Some(lang.as_str()));
        let face=faces.next()?;if faces.next().is_some(){return None;}
        let mut entries=face.children().filter(|n|n.has_tag_name((HH,"font")) && n.attribute("id")==Some(id));
        let font=entries.next()?;if entries.next().is_some(){return None;}
        fonts.insert(script.into(),attributes(font));
    }
    Some(json!({"char_pr_attributes":attributes(c),"font_ref_attributes":attributes(refs),"fonts":fonts,"bold":child(c,"bold").is_some(),"italic":child(c,"italic").is_some(),"font_availability_verified":false}))
}
// Only the independently authored five-property marker subset is certified.
fn marker_style_diagnostic(root:roxmltree::Node<'_, '_>,cid:&str)->Option<&'static str> {
    let c=definition(root,"charProperties","charPr",cid)?;
    if marker_binding(root,cid).is_none() {return Some("invalid_native_marker_font_reference");}
    let bad="unsupported_native_marker_character_style";
    let height=c.attribute("height").unwrap_or("");let color=c.attribute("textColor").unwrap_or("");
    if !height.parse::<u32>().is_ok_and(|v|(1..=100000).contains(&v)) || color.len()!=7 || !color.starts_with('#') || !color[1..].bytes().all(|b|b.is_ascii_hexdigit()) || !exact_attributes(c,&[("id",cid),("height",height),("textColor",color),("shadeColor","none"),("useFontSpace","0"),("useKerning","0"),("symMark","NONE"),("borderFillIDRef","2")]) {return Some(bad);}
    let scripts=["hangul","latin","hanja","japanese","other","symbol","user"];
    let mut seen=std::collections::BTreeSet::new();
    for n in c.children() {
        if !n.is_element() {if n.is_text() && n.text().unwrap_or("").trim().is_empty(){continue;}return Some(bad);}
        let tag=n.tag_name().name();
        if n.tag_name().namespace()!=Some(HH) || !seen.insert(tag) || n.children().next().is_some() {return Some(bad);}
        let valid=match tag {
            "fontRef"=>n.attributes().len()==7 && n.attributes().all(|a|a.namespace().is_none() && scripts.contains(&a.name())),
            "ratio"|"spacing"|"relSz"|"offset"=>{let value=if tag=="ratio" || tag=="relSz" {"100"}else{"0"};exact_attributes(n,&scripts.map(|s|(s,value)))},
            "bold"|"italic"=>n.attributes().len()==0,
            "underline"=>exact_attributes(n,&[("type","NONE"),("shape","SOLID"),("color","#000000")]),
            "strikeout"=>exact_attributes(n,&[("shape","NONE"),("color","#000000")]),
            "outline"=>exact_attributes(n,&[("type","NONE")]),
            "shadow"=>exact_attributes(n,&[("type","NONE"),("color","#C0C0C0"),("offsetX","10"),("offsetY","10")]),
            _=>false,
        };
        if !valid {return Some(bad);}
    }
    if ["fontRef","ratio","spacing","relSz","offset","underline","strikeout","outline","shadow"].iter().any(|t|!seen.contains(t)) {return Some(bad);}
    let mut family=None;
    let registry=child(child(root,"refList")?,"fontfaces")?;
    let refs=child(c,"fontRef")?;
    for script in scripts {
        let id=refs.attribute(script)?;let lang=script.to_ascii_uppercase();
        let face=registry.children().find(|n|n.has_tag_name((HH,"fontface")) && n.attribute("lang")==Some(lang.as_str()))?;
        let f=face.children().find(|n|n.has_tag_name((HH,"font")) && n.attribute("id")==Some(id))?;
        let name=f.attribute("face").unwrap_or("");
        if !exact_attributes(f,&[("id",id),("face",name),("type","TTF"),("isEmbedded","0")]) || f.children().next().is_some() || name.is_empty() || name.len()>256 || name.chars().any(|c|c.is_control()) || family.is_some_and(|v|v!=name) {return Some(bad);}
        family=Some(name);
    }
    None
}
/// Deliberately certifies only the authored level-zero, non-image bullet subset.
/// Other valid HWPX list models are unsupported, not silently normalized.
pub(crate) fn diagnostics(parts:&[(String,Vec<u8>)])->Vec<Value> {
    let mut out=Vec::new();
    let Some((_,bytes))=parts.iter().find(|(n,_)|n=="Contents/header.xml")else{return out};
    let Some(doc)=std::str::from_utf8(bytes).ok().and_then(|s|roxmltree::Document::parse(s).ok())else{return out};
    let root=doc.root_element();
    let mut error=|code:&str|out.push(json!({"severity":"error","code":code,"part":"Contents/header.xml"}));
    let groups:Vec<_>=doc.descendants().filter(|n|n.has_tag_name((HH,"bullets"))).collect();
    let mut ids=std::collections::BTreeSet::new();
    for group in &groups {
        let defs:Vec<_>=group.children().filter(|n|n.is_element()).collect();
        if groups.len()!=1 || child(root,"refList").and_then(|r|child(r,"bullets"))!=Some(*group) || !exact_attributes(*group,&[("itemCnt",&defs.len().to_string())]) {error("invalid_native_bullet_registry");}
        for b in defs {
            let id=b.attribute("id").unwrap_or("");
            if !b.has_tag_name((HH,"bullet")) || !id.parse::<u32>().is_ok_and(|v|v>0) || !ids.insert(id.to_string()) {error("invalid_native_bullet_registry");}
            let marker=b.attribute("char").unwrap_or("");
            if !["•","■"].contains(&marker) || !exact_attributes(b,&[("id",id),("char",marker),("useImage","0")]) || b.children().filter(|n|n.is_element()).count()!=1 || b.children().any(|n|n.is_text() && !n.text().unwrap_or("").trim().is_empty()) {error("unsupported_native_bullet_definition");}
            if let Some(ph)=child(b,"paraHead") {
                let cid=ph.attribute("charPrIDRef").unwrap_or("");
                if !exact_attributes(ph,&[("level","0"),("align","LEFT"),("useInstWidth","1"),("autoIndent","1"),("widthAdjust","0"),("textOffsetType","PERCENT"),("textOffset","50"),("numFormat","DIGIT"),("charPrIDRef",cid),("checkable","0")]) || ph.children().any(|n|n.is_element() || n.is_text() && !n.text().unwrap_or("").trim().is_empty()) {error("unsupported_native_bullet_definition");}
                if definition(root,"charProperties","charPr",cid).is_none() {error("invalid_native_bullet_character_reference");}
                if let Some(code)=marker_style_diagnostic(root,cid) {error(code);}
            }else{error("unsupported_native_bullet_definition");}
        }
    }
    let groups:Vec<_>=doc.descendants().filter(|n|n.has_tag_name((HH,"numberings"))).collect();
    let registered=groups.iter().flat_map(|g|g.children()).filter(|n|n.has_tag_name((HH,"numbering"))).count();
    if registered!=doc.descendants().filter(|n|n.has_tag_name((HH,"numbering"))).count() {error("invalid_native_numbering_registry");}
    let mut ids=std::collections::BTreeSet::new();
    for group in &groups {
        let defs:Vec<_>=group.children().filter(|n|n.is_element()).collect();
        if groups.len()!=1 || child(root,"refList").and_then(|r|child(r,"numberings"))!=Some(*group) || !exact_attributes(*group,&[("itemCnt",&defs.len().to_string())]) {error("invalid_native_numbering_registry");}
        for n in defs {
            let id=n.attribute("id").unwrap_or("");
            if !n.has_tag_name((HH,"numbering")) || !id.parse::<u32>().is_ok_and(|v|v>0) || !ids.insert(id.to_string()) {error("invalid_native_numbering_registry");}
            if !exact_attributes(n,&[("id",id),("start","0")]) || n.children().filter(|c|c.is_element()).count()!=1 || n.children().any(|c|c.is_text() && !c.text().unwrap_or("").trim().is_empty()) {error("unsupported_native_numbering_definition");}
            if let Some(ph)=child(n,"paraHead") {
                let cid=ph.attribute("charPrIDRef").unwrap_or("");let start=ph.attribute("start").unwrap_or("");
                if !start.parse::<u32>().is_ok_and(|v|(1..=65535).contains(&v)) || !exact_attributes(ph,&[("start",start),("level","1"),("align","LEFT"),("useInstWidth","1"),("autoIndent","1"),("widthAdjust","0"),("textOffsetType","PERCENT"),("textOffset","50"),("numFormat","DIGIT"),("charPrIDRef",cid),("checkable","0")]) || ph.children().any(|c|!c.is_text()) || ph.text()!=Some("^1.") {error("unsupported_native_numbering_definition");}
                if definition(root,"charProperties","charPr",cid).is_none() {error("invalid_native_numbering_character_reference");}
                if let Some(code)=marker_style_diagnostic(root,cid) {error(code);}
            }else{error("unsupported_native_numbering_definition");}
        }
    }
    for p in doc.descendants().filter(|n|n.has_tag_name((HH,"paraPr"))) {
        let headings:Vec<_>=p.children().filter(|n|n.has_tag_name((HH,"heading"))).collect();
        if headings.len()>1 {error("invalid_native_list_heading");continue;}
        for h in headings {
            if h.attribute("type")==Some("NONE"){continue;}
            let id=h.attribute("idRef").unwrap_or("");
            let kind=h.attribute("type").unwrap_or("");
            if !["BULLET","NUMBER"].contains(&kind) || kind=="NUMBER" && definition(root,"numberings","numbering",id).is_none() || !exact_attributes(h,&[("type",kind),("idRef",id),("level","0")]) || h.children().any(|n|n.is_element() || n.is_text() && !n.text().unwrap_or("").trim().is_empty()) {error("unsupported_native_list_heading");}
            if h.attribute("type")==Some("BULLET") && definition(root,"bullets","bullet",id).is_none() {error("invalid_native_bullet_reference");}
            if kind=="NUMBER" && definition(root,"numberings","numbering",id).is_none() {error("invalid_native_numbering_reference");}
        }
    }
    for (name,bytes) in parts.iter().filter(|(n,_)|n.starts_with("Contents/section") && n.ends_with(".xml")) {
        let Some(section)=std::str::from_utf8(bytes).ok().and_then(|s|roxmltree::Document::parse(s).ok())else{continue};
        let hp=crate::HP;
        for p in section.descendants().filter(|n|n.has_tag_name((hp,"p"))) {
            let Some(pr)=p.attribute("paraPrIDRef").and_then(|id|definition(root,"paraProperties","paraPr",id))else{continue};
            if child(pr,"heading").and_then(|h|h.attribute("type"))!=Some("NUMBER"){continue;}
            let mut valid=p.parent()==Some(section.root_element()) && p.attribute("pageBreak")==Some("0") && p.attribute("columnBreak")==Some("0") && p.attribute("merged")==Some("0");
            valid &= child(pr,"breakSetting").is_some_and(|b|["keepWithNext","keepLines","pageBreakBefore"].iter().all(|a|b.attribute(*a)==Some("0")));
            let mut nonblank=false;
            for run in p.children().filter(|n|n.is_element()) {
                if !run.has_tag_name((hp,"run")) || run.attribute("charPrIDRef").and_then(|id|definition(root,"charProperties","charPr",id)).is_none() {valid=false;}
                for c in run.children().filter(|n|n.is_element()) {
                    if c.has_tag_name((hp,"t")) {
                        valid &= !c.children().any(|n|!n.is_text());
                        nonblank |= c.text().is_some_and(|t|!t.trim().is_empty());
                    }else {
                        // create's initial structural setup is a separate non-text run.
                        let initial=p==section.root_element().children().find(|n|n.is_element()).unwrap_or(p) && run==p.children().find(|n|n.is_element()).unwrap_or(run);
                        let setup:Vec<_>=run.children().filter(|n|n.is_element()).collect();
                        valid &= initial && setup.len()==2 && setup[0].has_tag_name((hp,"secPr")) && setup[1].has_tag_name((hp,"ctrl")) && setup[1].children().filter(|n|n.is_element()).count()==1 && setup[1].children().any(|n|n.has_tag_name((hp,"colPr")));
                    }
                }
            }
            if !valid || !nonblank {out.push(json!({"severity":"error","code":"unsupported_native_numbering_paragraph","part":name,"paragraph_id":p.attribute("id")}));}
        }
    }
    out
}
pub(crate) fn bind(paragraphs:&mut [ObjectNode],parts:&[(String,Vec<u8>)],revision:&str) {
    let supported=diagnostics(parts).is_empty();
    let Some((_,bytes))=parts.iter().find(|(n,_)|n=="Contents/header.xml")else{return};
    let Some(doc)=std::str::from_utf8(bytes).ok().and_then(|s|roxmltree::Document::parse(s).ok())else{return};
    let root=doc.root_element();
    for p in paragraphs {
        let Some(pid)=p.attributes.get("paraPrIDRef")else{continue};
        let Some(para)=definition(root,"paraProperties","paraPr",pid)else{continue};
        let Some(heading)=child(para,"heading")else{continue};
        if heading.attribute("type")==Some("NONE"){continue;}
        let mut value=json!({"binding_status":"unresolved_or_unsupported","revision":revision,"header_part":"Contents/header.xml","para_pr_id":pid,"heading_type":heading.attribute("type"),"definition_id":heading.attribute("idRef"),"level":heading.attribute("level")});
        if supported && heading.attribute("type")==Some("BULLET") && heading.attribute("level")==Some("0") {
            if let Some(b)=heading.attribute("idRef").and_then(|id|definition(root,"bullets","bullet",id)) {
                if let Some(ph)=child(b,"paraHead") {
                    if b.attribute("useImage")==Some("0") && ph.attribute("level")==Some("0") && ph.attribute("charPrIDRef").and_then(|id|definition(root,"charProperties","charPr",id)).is_some() {
                        value["binding_status"]=json!("bound_from_saved_xml");
                        value["marker"]=json!(b.attribute("char"));
                        value["char_pr_id"]=json!(ph.attribute("charPrIDRef"));
                        value["definition_attributes"]=json!(b.attributes().map(|a|(a.name(),a.value())).collect::<std::collections::BTreeMap<_,_>>());
                        value["para_head_attributes"]=json!(ph.attributes().map(|a|(a.name(),a.value())).collect::<std::collections::BTreeMap<_,_>>());
                        value["counter_state"]=Value::Null; // Bullets have no start/current counter.
                    }
                }
            }
        }
        if supported && heading.attribute("type")==Some("NUMBER") && heading.attribute("level")==Some("0") {
            if let Some(n)=heading.attribute("idRef").and_then(|id|definition(root,"numberings","numbering",id)) {
                if let Some(ph)=child(n,"paraHead") {
                    value["binding_status"]=json!("bound_from_saved_xml");
                    value["char_pr_id"]=json!(ph.attribute("charPrIDRef"));
                    value["definition_attributes"]=json!(n.attributes().map(|a|(a.name(),a.value())).collect::<std::collections::BTreeMap<_,_>>());
                    value["para_head_attributes"]=json!(ph.attributes().map(|a|(a.name(),a.value())).collect::<std::collections::BTreeMap<_,_>>());
                    value["format_text"]=json!(ph.text());
                    value["definition_start"]=json!(n.attribute("start"));
                    value["level_start"]=json!(ph.attribute("start"));
                    value["counter_state"]=Value::Null; // Actual renderer state is not computed.
                }
            }
        }
        if value["binding_status"]=="bound_from_saved_xml" {
            if let Some(cid)=value["char_pr_id"].as_str() {
                if let Some(style)=marker_binding(root,cid) {value["marker_character_style"]=style;}
            }
        }
        p.native_list=Some(value);
    }
}
