//! Scoped, preservation-first PNG/JPEG editing. Same pinned Hancom PictureType/item
//! model as inline.rs; no external resource reads and no opaque picture rebuild.
use crate::{Package,Result,HP};
use crate::styles::{patch,standalone,signature,set_attr};
use crate::package_model::{HC,OPF};
use serde::Deserialize;
use sha2::{Digest,Sha256};
use std::collections::{BTreeMap,BTreeSet};
#[derive(Deserialize)]
#[serde(untagged,deny_unknown_fields)]
pub enum ImageReplace {
    Png {revision:String,target:String,expected_binary_item_id:String,expected_sha256:String,png_hex:String},
    Jpeg {revision:String,target:String,expected_binary_item_id:String,expected_sha256:String,format:crate::inline::JpegFormat,jpeg_hex:String},
}
struct Replacement {revision:String,target:String,expected_binary_item_id:String,expected_sha256:String,data:String,jpeg:bool}
impl ImageReplace {
    fn payload(self)->Replacement {match self {
        Self::Png{revision,target,expected_binary_item_id,expected_sha256,png_hex}=>Replacement{revision,target,expected_binary_item_id,expected_sha256,data:png_hex,jpeg:false},
        Self::Jpeg{revision,target,expected_binary_item_id,expected_sha256,format:_,jpeg_hex}=>Replacement{revision,target,expected_binary_item_id,expected_sha256,data:jpeg_hex,jpeg:true},
    }}
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageInsert {revision:String,target:String,expected_run_count:usize,position:usize,char_pr_id:u32,image:crate::inline::ImageSpec}
fn parse(xml:&str)->Result<roxmltree::Document<'_>> {roxmltree::Document::parse(xml).map_err(|_|"invalid_xml")}
pub(crate) fn supported_picture(xml:&str,pic:roxmltree::Node)->Result<(u32,String,u32,u32)> {
    let id=pic.attribute("id").and_then(|v|v.parse().ok()).ok_or("unsupported_picture_edit")?;
    let size=pic.children().find(|n|n.has_tag_name((HP,"sz"))).ok_or("unsupported_picture_edit")?;
    let w=size.attribute("width").and_then(|v|v.parse().ok()).ok_or("unsupported_picture_edit")?;
    let h=size.attribute("height").and_then(|v|v.parse().ok()).ok_or("unsupported_picture_edit")?;
    let item=pic.children().find(|n|n.has_tag_name((HC,"img"))).and_then(|n|n.attribute("binaryItemIDRef")).ok_or("unsupported_picture_edit")?;
    if item.is_empty() || !item.bytes().all(|b|b.is_ascii_alphanumeric() || b==b'_' || b==b'-') {return Err("unsupported_picture_edit");}
    let original=standalone(xml,pic)?;let o=parse(&original)?;
    let wrapper=format!("<root xmlns:hp=\"{HP}\" xmlns:hc=\"{HC}\">{}</root>",crate::inline::picture(id,item,w,h));
    let d=parse(&wrapper)?;let expected=standalone(&wrapper,d.root_element().first_element_child().ok_or("invalid_xml")?)?;let e=parse(&expected)?;
    if signature(o.root_element())!=signature(e.root_element()) {return Err("unsupported_picture_edit");}
    Ok((id,item.to_string(),w,h))
}
impl Package {
    fn media_xml(&self,name:&str)->Result<&str> {
        std::str::from_utf8(&self.parts.iter().find(|(n,_)|n==name).ok_or("missing_media_package_part")?.1).map_err(|_|"invalid_xml")
    }
    fn admit_media(&self,revision:&str)->Result<()> {
        if revision!=self.inspection.revision {return Err("stale_document");}
        if self.validate_supported()["valid_supported_structure"]!=true {return Err("unsupported_media_package");}
        Ok(())
    }
    fn append_image(&self,raw:Vec<u8>,changed:&mut BTreeMap<String,Vec<u8>>)->Result<String> {
        let (extension,mime)=crate::inline::image_kind(&raw);
        let xml=self.media_xml("Contents/content.hpf")?;let d=parse(xml)?;
        let manifests:Vec<_>=d.root_element().children().filter(|n|n.has_tag_name((OPF,"manifest"))).collect();
        if manifests.len()!=1 {return Err("unsupported_media_manifest");}
        let manifest=manifests[0];
        let mut reserved=BTreeSet::new();
        for (name,b) in &self.parts {if name.ends_with(".xml") || name.ends_with(".hpf") {
            let s=std::str::from_utf8(b).map_err(|_|"invalid_xml")?;let p=parse(s)?;
            for n in p.descendants() {for a in n.attributes() {reserved.insert(a.value().to_ascii_lowercase());}}
        }}
        let occupied:BTreeSet<_>=self.parts.iter().map(|(n,_)|n.to_ascii_lowercase()).collect();
        let (item,path)=(1..=10000).map(|i|{let item=format!("image_edit{i}");let path=format!("BinData/{item}.{extension}");(item,path)})
            .find(|(id,path)|!reserved.contains(id) && !reserved.contains(&path.to_ascii_lowercase()) && !occupied.contains(&path.to_ascii_lowercase())).ok_or("excess_media_ids")?;
        let tag=format!("<opf:item xmlns:opf=\"{OPF}\" id=\"{item}\" href=\"{path}\" media-type=\"{mime}\" isEmbeded=\"1\"/>");
        let raw_manifest=&xml[manifest.range()];let close=raw_manifest.rfind("</").ok_or("unsupported_media_manifest")?;
        let position=manifest.range().start+close;
        changed.insert("Contents/content.hpf".into(),patch(xml,vec![(position..position,tag)])?.into_bytes());
        changed.insert(path,raw);Ok(item)
    }
    pub fn image_replace_bytes(&self,spec:ImageReplace)->Result<crate::Prepared> {
        let spec=spec.payload();
        self.admit_media(&spec.revision)?;
        if spec.expected_sha256.len()!=64 || !spec.expected_sha256.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {return Err("invalid_expected_sha256");}
        let (part,index)=spec.target.rsplit_once("#pic").ok_or("unknown_target")?;
        if !self.inspection.images.iter().any(|n|n["id"]==spec.target) {return Err("unknown_target");}
        let xml=self.media_xml(part)?;let d=parse(xml)?;
        let pic=d.descendants().filter(|n|n.has_tag_name((HP,"pic"))).nth(index.parse::<usize>().map_err(|_|"unknown_target")?).ok_or("unknown_target")?;
        let (_,item,w,h)=supported_picture(xml,pic)?;
        if pic.ancestors().any(|n|n.has_tag_name((HP,"tc")) && n.attribute("protect").is_some_and(|v|v!="0")) {return Err("protected_media_target");}
        if item!=spec.expected_binary_item_id {return Err("stale_target");}
        let manifest=parse(self.media_xml("Contents/content.hpf")?)?;
        let items:Vec<_>=manifest.descendants().filter(|n|n.has_tag_name((OPF,"item")) && n.attribute("id")==Some(item.as_str())).collect();
        let mime=if spec.jpeg {"image/jpeg"} else {"image/png"};
        if items.len()!=1 || items[0].attribute("media-type")!=Some(mime) || items[0].attribute("isEmbeded")!=Some("1") {return Err("unsupported_media_manifest");}
        let path=items[0].attribute("href").ok_or("unsupported_media_manifest")?;
        if !path.starts_with("BinData/") || !crate::package_model::safe_part(path) {return Err("unsafe_media_path");}
        let old=&self.parts.iter().find(|(n,_)|n==path).ok_or("missing_media_resource")?.1;
        if format!("{:x}",Sha256::digest(old))!=spec.expected_sha256 {return Err("stale_target");}
        let image=if spec.jpeg {crate::inline::ImageSpec::Jpeg(crate::inline::JpegImageSpec{format:crate::inline::JpegFormat::Jpeg,jpeg_hex:spec.data,width:w,height:h})}
            else {crate::inline::ImageSpec::Png(crate::inline::PngImageSpec{png_hex:spec.data,width:w,height:h})};
        let raw=crate::inline::image_data(&image)?;
        let mut changed=BTreeMap::new();
        if old!=&raw {
            // Always COW, even if apparently unshared: preserve opaque references
            // and unreferenced original resources without claiming safe removal.
            let new_id=self.append_image(raw,&mut changed)?;
            let img=pic.children().find(|n|n.has_tag_name((HC,"img"))).ok_or("unsupported_picture_edit")?;
            changed.insert(part.to_string(),patch(xml,vec![(img.range(),set_attr(xml,img,"binaryItemIDRef",&new_id)?)])?.into_bytes());
        }
        self.prepare_changed(changed,true)
    }
    pub fn image_insert_bytes(&self,spec:ImageInsert)->Result<crate::Prepared> {
        self.admit_media(&spec.revision)?;
        let (part,index)=spec.target.rsplit_once("#p").ok_or("unknown_target")?;
        if !self.inspection.paragraphs.iter().any(|n|n.id==spec.target) {return Err("unknown_target");}
        let xml=self.media_xml(part)?;let d=parse(xml)?;
        let p=d.descendants().filter(|n|n.has_tag_name((HP,"p"))).nth(index.parse::<usize>().map_err(|_|"unknown_target")?).ok_or("unknown_target")?;
        if p.attributes().any(|a|a.namespace().is_some() || !["id","paraPrIDRef","styleIDRef","pageBreak","columnBreak","merged"].contains(&a.name())) ||
            p.children().any(|n|!(n.is_text() && n.text().unwrap_or("").trim().is_empty()) && !n.has_tag_name((HP,"run"))) {return Err("unsupported_insertion_paragraph");}
        let runs:Vec<_>=p.children().filter(|n|n.has_tag_name((HP,"run"))).collect();
        if runs.len()!=spec.expected_run_count {return Err("stale_target");}
        if runs.is_empty() || spec.position>runs.len() || runs.len()>=1000 {return Err("invalid_insertion_position");}
        if runs.iter().skip(spec.position).any(|r|r.descendants().any(|n|n.has_tag_name((HP,"secPr")))) {return Err("insertion_before_section_setup");}
        let position=if spec.position<runs.len() {runs[spec.position].range().start}else{runs.last().ok_or("invalid_insertion_position")?.range().end};
        // Only admit a direct-run boundary outside fields. A picture must not
        // accidentally acquire a hyperlink/protected-field owner.
        let mut stack=Vec::new();
        for n in d.descendants().filter(|n|n.range().start<position) {
            if n.has_tag_name((HP,"fieldBegin")) {
                if n.attribute("type")!=Some("HYPERLINK") {return Err("unsupported_insertion_field");}
                stack.push(n.attribute("id").ok_or("unsupported_insertion_field")?);
            } else if n.has_tag_name((HP,"fieldEnd")) && stack.pop()!=n.attribute("beginIDRef") {return Err("unsupported_insertion_field");}
        }
        if !stack.is_empty() {return Err("insertion_inside_field");}
        let header=parse(self.media_xml("Contents/header.xml")?)?;
        if !header.descendants().any(|n|n.has_tag_name((crate::styles::HH,"charPr")) && n.attribute("id").and_then(|v|v.parse::<u32>().ok())==Some(spec.char_pr_id)) {return Err("unresolved_character_reference");}
        let raw=crate::inline::image_data(&spec.image)?;
        let number=|n:roxmltree::Node,key:&str|->Result<u32>{n.attribute(key).ok_or("unsupported_page_geometry")?.parse().map_err(|_|"unsupported_page_geometry")};
        if p.ancestors().any(|n|n.has_tag_name((HP,"tc")) && n.attribute("protect").is_some_and(|v|v!="0")) {return Err("protected_insertion_target");}
        let body=if let Some(cell)=p.ancestors().find(|n|n.has_tag_name((HP,"tc"))) {
            let list=cell.children().find(|n|n.has_tag_name((HP,"subList"))).ok_or("unsupported_insertion_paragraph")?;
            if ["linkListIDRef","linkListNextIDRef","hasTextRef","hasNumRef"].iter().any(|k|list.attribute(*k).is_some_and(|v|!v.is_empty() && v!="0")) {return Err("protected_insertion_target");}
            let size=cell.children().find(|n|n.has_tag_name((HP,"cellSz"))).ok_or("unsupported_page_geometry")?;
            let m=cell.children().find(|n|n.has_tag_name((HP,"cellMargin"))).ok_or("unsupported_page_geometry")?;
            number(size,"width")?.checked_sub(number(m,"left")?.saturating_add(number(m,"right")?)).ok_or("unsupported_page_geometry")?
        }else{
            let pages:Vec<_>=d.descendants().filter(|n|n.has_tag_name((HP,"pagePr"))).collect();
            if pages.len()!=1 {return Err("unsupported_page_geometry");}
            if d.descendants().any(|n|n.has_tag_name((HP,"colPr")) && n.attribute("colCount")!=Some("1")) {return Err("unsupported_page_geometry");}
            let margin=pages[0].children().find(|n|n.has_tag_name((HP,"margin"))).ok_or("unsupported_page_geometry")?;
            number(pages[0],"width")?.checked_sub(number(margin,"left")?.saturating_add(number(margin,"right")?)).ok_or("unsupported_page_geometry")?
        };
        if spec.image.width()>body {return Err("image_body_width_exceeded");}
        let id=self.next_object_id()?;
        let mut changed=BTreeMap::new();let item=self.append_image(raw,&mut changed)?;
        let picture=crate::inline::picture(id,&item,spec.image.width(),spec.image.height());
        let run=format!("<hp:run xmlns:hp=\"{HP}\" xmlns:hc=\"{HC}\" charPrIDRef=\"{}\">{picture}</hp:run>",spec.char_pr_id);
        changed.insert(part.to_string(),patch(xml,vec![(position..position,run)])?.into_bytes());
        self.prepare_changed(changed,true)
    }
}
