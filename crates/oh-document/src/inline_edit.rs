//! Revision-bound edits for the generated inline subset. Opaque pictures reject.
use crate::{Package,Result,HP,escape_xml};
use crate::styles::patch;
use crate::package_model::HC;
use serde::Deserialize;
use std::collections::BTreeMap;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageEdit {revision:String,target:String,width:u32,height:u32}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkEdit {revision:String,target:String,url:String}
impl Package {
    pub fn image_bytes(&self,spec:ImageEdit)->Result<crate::Prepared> {
        if spec.revision!=self.inspection.revision {return Err("stale_document");}
        if spec.width==0 || spec.height==0 || spec.width>1000000 || spec.height>1000000 {return Err("invalid_image_geometry");}
        let (part,index)=spec.target.rsplit_once("#pic").ok_or("unknown_target")?;
        let index=index.parse::<usize>().map_err(|_|"unknown_target")?;
        if !self.inspection.images.iter().any(|n|n["id"]==spec.target) {return Err("unknown_target");}
        let xml=std::str::from_utf8(&self.parts.iter().find(|(n,_)|n==part).ok_or("unknown_target")?.1).map_err(|_|"invalid_xml")?;
        let doc=roxmltree::Document::parse(xml).map_err(|_|"invalid_xml")?;
        let pic=doc.descendants().filter(|n|n.has_tag_name((HP,"pic"))).nth(index).ok_or("unknown_target")?;
        let (id,item,w,h)=crate::media_edit::supported_picture(xml,pic)?;
        let mut changed=BTreeMap::new();
        if w!=spec.width || h!=spec.height {
            let candidate=format!("<hp:pic xmlns:hp=\"{HP}\" xmlns:hc=\"{HC}\"{}",crate::inline::picture(id,&item,spec.width,spec.height).strip_prefix("<hp:pic").ok_or("invalid_xml")?);
            changed.insert(part.to_string(),patch(xml,vec![(pic.range(),candidate)])?.into_bytes());
        }
        self.prepare_changed(changed,false)
    }
    pub fn link_bytes(&self,spec:LinkEdit)->Result<crate::Prepared> {
        if spec.revision!=self.inspection.revision {return Err("stale_document");}
        if !crate::inline::url_ok(&spec.url) {return Err("unsafe_hyperlink");}
        let (part,index)=spec.target.rsplit_once("#link").ok_or("unknown_target")?;
        let index=index.parse::<usize>().map_err(|_|"unknown_target")?;
        if !self.inspection.hyperlinks.iter().any(|n|n["id"]==spec.target) {return Err("unknown_target");}
        let xml=std::str::from_utf8(&self.parts.iter().find(|(n,_)|n==part).ok_or("unknown_target")?.1).map_err(|_|"invalid_xml")?;
        let doc=roxmltree::Document::parse(xml).map_err(|_|"invalid_xml")?;
        let field=doc.descendants().filter(|n|n.has_tag_name((HP,"fieldBegin")) && n.attribute("type")==Some("HYPERLINK")).nth(index).ok_or("unknown_target")?;
        let mut edits=Vec::new();
        let rendered=crate::inline::hyperlink(1,0,"",&spec.url)?;
        let wrapper=format!("<root xmlns:hp=\"{HP}\">{rendered}</root>");
        let rendered_doc=roxmltree::Document::parse(&wrapper).map_err(|_|"invalid_xml")?;
        for key in ["Command","Path"] {
            let nodes:Vec<_>=field.descendants().filter(|n|n.has_tag_name((HP,"stringParam")) && n.attribute("name")==Some(key)).collect();
            if nodes.len()!=1 || nodes[0].children().any(|n|!n.is_text()) {return Err("unsupported_hyperlink_edit");}
            let node=nodes[0];
            let text=rendered_doc.descendants().find(|n|n.has_tag_name((HP,"stringParam")) && n.attribute("name")==Some(key)).and_then(|n|n.text()).ok_or("invalid_xml")?;
            if node.text().unwrap_or("")!=text {
                let raw=&xml[node.range()];let end=crate::tag_end(raw).ok_or("invalid_xml")?;
                if raw[..end].ends_with("/>") {return Err("unsupported_hyperlink_edit");}
                let close=raw.rfind("</").ok_or("invalid_xml")?;
                edits.push((node.range(),format!("{}{}{}",&raw[..end],escape_xml(text),&raw[close..])));
            }
        }
        let mut changed=BTreeMap::new();if !edits.is_empty() {changed.insert(part.to_string(),patch(xml,edits)?.into_bytes());}
        self.prepare_changed(changed,false)
    }
}
