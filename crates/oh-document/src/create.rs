//! New editable package generation from a bounded semantic specification.
//! Public OWPML element model; NOT native layout or complete schema certification.
use crate::{Package,Prepared,Result,HP,escape_xml,valid_xml_char};
use serde::Deserialize;
use std::{collections::BTreeMap,io::{Cursor,Write}};
mod compound;
mod appearance;
#[derive(Deserialize,Clone,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
struct BulletList {kind:String,marker:String,#[serde(default,deserialize_with="marker_style")] marker_style:Option<MarkerStyle>}
#[derive(Deserialize,Clone,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
struct Numbering {group:String,start:u32,format:String,#[serde(default,deserialize_with="marker_style")] marker_style:Option<MarkerStyle>}
#[derive(Deserialize,Clone,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
struct MarkerStyle {height:u32,color:String,font_family:String,bold:bool,italic:bool}
// Omission is legacy charPr0; explicit null is not an opt-in style.
fn marker_style<'de,D:serde::Deserializer<'de>>(d:D)->std::result::Result<Option<MarkerStyle>,D::Error> {
    MarkerStyle::deserialize(d).map(Some)
}
#[derive(Clone,PartialEq,Eq)]
enum NativeHeading {Bullet(usize),Number(usize)}
const HH:&str="http://www.hancom.co.kr/hwpml/2011/head";
const HC:&str="http://www.hancom.co.kr/hwpml/2011/core";
fn default_height()->u32 {1000}
fn black()->String {"#000000".into()}
fn left()->String {"LEFT".into()}
fn font()->String {"함초롬바탕".into()}
fn one()->usize {1}

fn center()->String {"CENTER".into()}
#[derive(Deserialize,Clone,PartialEq,Eq)]
#[serde(default,deny_unknown_fields)]
struct ParaProps {indent:i32,left:u32,right:u32,space_before:u32,space_after:u32,line_spacing_percent:Option<u32>,line_spacing:Option<crate::line_spacing::LineSpacing>,keep_with_next:bool,keep_lines:bool}
impl Default for ParaProps {fn default()->Self {Self{indent:0,left:0,right:0,space_before:0,space_after:0,line_spacing_percent:None,line_spacing:None,keep_with_next:false,keep_lines:false}}}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CellMargins {left:u32,right:u32,top:u32,bottom:u32}
impl Default for CellMargins {fn default()->Self {Self{left:510,right:510,top:141,bottom:141}}}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSpec {
    #[serde(default)] page:Page,
    blocks:Vec<Block>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page { width:u32,height:u32,left:u32,right:u32,top:u32,bottom:u32 }
impl Default for Page {fn default()->Self {Self{width:59528,height:84186,left:8504,right:8504,top:5668,bottom:4252}}}
#[derive(Deserialize)]
#[serde(tag="type",rename_all="snake_case",deny_unknown_fields)]
enum Block {
    Paragraph {runs:Vec<Run>,#[serde(default="left")] align:String,#[serde(default)] page_break:bool,#[serde(default)] properties:ParaProps,#[serde(default)] list:Option<BulletList>,#[serde(default)] numbering:Option<Numbering>},
    Table {column_widths:Vec<u32>,row_heights:Vec<u32>,rows:Vec<Vec<Cell>>},
}
#[derive(Deserialize,Clone,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
struct Run {
    text:String,
    #[serde(default="default_height")] height:u32,
    #[serde(default)] bold:bool,
    #[serde(default)] italic:bool,
    #[serde(default="font")] font_family:String,
    #[serde(default)] font_families:BTreeMap<String,String>,
    #[serde(default="black")] color:String,
    #[serde(default)] image:Option<crate::inline::ImageSpec>,
    #[serde(default)] hyperlink:Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cell {text:Option<String>,runs:Option<Vec<Run>>,blocks:Option<Vec<Block>>,#[serde(default="one")] rowspan:usize,#[serde(default="one")] colspan:usize,#[serde(default)] margins:CellMargins,#[serde(default="center")] vertical_align:String,appearance:Option<appearance::Appearance>}
struct Builder { chars:Vec<(u32,bool,String,bool,[usize;7])>,fonts:Vec<String>,paras:Vec<(String,ParaProps,Option<NativeHeading>)>,bullets:Vec<(String,usize)>,numberings:Vec<(Numbering,usize)>,next_id:u32,media:Vec<Vec<u8>>,cell_appearances:Vec<appearance::Appearance> }
impl Builder {
    // Shared native character/font registry for body runs and independent markers.
    fn character(&mut self,height:u32,bold:bool,color:&str,italic:bool,family:&str,families:&BTreeMap<String,String>)->Result<usize> {
        if !(1..=100000).contains(&height) || color.len()!=7 || !color.starts_with('#') || !color[1..].bytes().all(|b|b.is_ascii_hexdigit()) {return Err("invalid_character_properties");}
        let mut register=|name:&str|->Result<usize> {
            if name.is_empty() || name.len()>256 || !name.chars().all(valid_xml_char) || name.chars().any(|c|c.is_control()) {return Err("invalid_font_family");}
            Ok(if let Some(i)=self.fonts.iter().position(|f|f==name) {i}else{self.fonts.push(name.to_string());self.fonts.len()-1})
        };
        let mut fids=[register(family)?;7];
        for (lang,name) in families {
            let index=["hangul","latin","hanja","japanese","other","symbol","user"].iter().position(|s|*s==lang).ok_or("unknown_font_script")?;
            fids[index]=register(name)?;
        }
        let style=(height,bold,color.to_ascii_uppercase(),italic,fids);
        Ok(if let Some(i)=self.chars.iter().position(|c|c==&style) {i}else{self.chars.push(style);self.chars.len()-1})
    }
    fn marker_character(&mut self,style:Option<&MarkerStyle>)->Result<usize> {
        match style {None=>Ok(0),Some(s)=>self.character(s.height,s.bold,&s.color,s.italic,&s.font_family,&BTreeMap::new())}
    }
    fn cell_appearance(&mut self,mut a:appearance::Appearance)->Result<usize> {
        a.normalize()?;
        if a==appearance::Appearance::default() {return Ok(2);}
        if let Some(i)=self.cell_appearances.iter().position(|v|v==&a) {return Ok(i+3);}
        if self.cell_appearances.len()>=9998 {return Err("excess_style_definitions");}
        self.cell_appearances.push(a);Ok(self.cell_appearances.len()+2)
    }
    fn id(&mut self)->u32 {let id=self.next_id;self.next_id+=1;id}
    fn para(&mut self,runs:Vec<Run>,align:&str)->Result<String> {
        self.para_with(runs,align,ParaProps::default(),None,None)
    }
    fn para_with(&mut self,runs:Vec<Run>,align:&str,mut properties:ParaProps,list:Option<BulletList>,numbering:Option<Numbering>)->Result<String> {
        if !["LEFT","RIGHT","CENTER","JUSTIFY","DISTRIBUTE","DISTRIBUTE_SPACE"].contains(&align) || runs.is_empty() || runs.len()>1000 {return Err("invalid_paragraph");}
        if properties.indent.unsigned_abs()>1000000 || [properties.left,properties.right,properties.space_before,properties.space_after].iter().any(|v|*v>1000000) {return Err("invalid_paragraph_measure");}
        crate::line_spacing::resolve(properties.line_spacing_percent,properties.line_spacing.as_ref())?;
        if properties.line_spacing_percent==Some(160) {properties.line_spacing_percent=None;}
        if list.is_some() && numbering.is_some() {return Err("conflicting_native_list_models");}
        if numbering.is_some() && (runs.iter().any(|r|r.image.is_some() || r.hyperlink.is_some()) || !runs.iter().any(|r|!r.text.trim().is_empty()) || properties.keep_with_next || properties.keep_lines) {return Err("unsupported_native_list_paragraph");}
        let bullet=if let Some(list)=list {
            if list.kind!="bullet" || !["•","■"].contains(&list.marker.as_str()) {return Err("unsupported_native_list");}
            if runs.iter().any(|r|r.image.is_some() || r.hyperlink.is_some()) || !runs.iter().any(|r|!r.text.trim().is_empty()) || properties.keep_with_next || properties.keep_lines {return Err("unsupported_native_list_paragraph");}
            let cid=self.marker_character(list.marker_style.as_ref())?;
            let key=(list.marker,cid);
            let id=if let Some(i)=self.bullets.iter().position(|m|m==&key) {i+1}else{self.bullets.push(key);self.bullets.len()};Some(NativeHeading::Bullet(id))
        }else if let Some(n)=numbering {
            if n.group.is_empty() || n.group.len()>64 || !n.group.bytes().all(|c|c.is_ascii_alphanumeric() || c==b'_' || c==b'-') || !(1..=65535).contains(&n.start) || n.format!="decimal-dot" {return Err("unsupported_native_numbering");}
            let cid=self.marker_character(n.marker_style.as_ref())?;
            let id=if let Some(i)=self.numberings.iter().position(|(v,_)|v.group==n.group) {
                let (old,old_cid)=&self.numberings[i];
                if old.start!=n.start || old.format!=n.format || *old_cid!=cid {return Err("conflicting_native_numbering_group");}i+1
            }else{self.numberings.push((n,cid));self.numberings.len()};Some(NativeHeading::Number(id))
        }else{None};
        let style=(align.to_string(),properties,bullet);
        let pid=if let Some(i)=self.paras.iter().position(|s| s==&style) {i} else {self.paras.push(style);self.paras.len()-1};
        let mut content=String::new();
        for run in runs {
            if !run.text.chars().all(valid_xml_char) {return Err("invalid_xml_character");}
            let cid=self.character(run.height,run.bold,&run.color,run.italic,&run.font_family,&run.font_families)?;
            if let Some(image)=run.image {
                if !run.text.is_empty() || run.hyperlink.is_some() {return Err("invalid_image_run");}
                let data=crate::inline::image_data(&image)?;
                let index=if let Some(i)=self.media.iter().position(|v|v==&data) {i} else {self.media.push(data);self.media.len()-1};
                let id=self.id();let item=format!("image{}",index+1);
                let pic=crate::inline::picture(id,&item,image.width(),image.height());
                content.push_str(&format!("<hp:run charPrIDRef=\"{cid}\">{pic}</hp:run>"));
            } else if let Some(url)=run.hyperlink {
                let id=self.id();content.push_str(&crate::inline::hyperlink(id,cid,&run.text,&url)?);
            } else {
                content.push_str(&format!("<hp:run charPrIDRef=\"{cid}\"><hp:t>{}</hp:t></hp:run>",escape_xml(&run.text)));
            }
        }
        let id=self.id();
        Ok(format!("<hp:p id=\"{id}\" paraPrIDRef=\"{pid}\" styleIDRef=\"0\" pageBreak=\"0\" columnBreak=\"0\" merged=\"0\">{content}</hp:p>"))
    }
    fn header(&self)->String {
        let mut chars=String::new();
        for (i,(height,bold,color,italic,fids)) in self.chars.iter().enumerate() {
            chars.push_str(&format!("<hh:charPr id=\"{i}\" height=\"{height}\" textColor=\"{color}\" shadeColor=\"none\" useFontSpace=\"0\" useKerning=\"0\" symMark=\"NONE\" borderFillIDRef=\"2\">"));
            chars.push_str(&format!("<hh:fontRef hangul=\"{}\" latin=\"{}\" hanja=\"{}\" japanese=\"{}\" other=\"{}\" symbol=\"{}\" user=\"{}\"/>",fids[0],fids[1],fids[2],fids[3],fids[4],fids[5],fids[6]));
            for (tag,v) in [("ratio",100),("spacing",0),("relSz",100),("offset",0)] {
                chars.push_str(&format!("<hh:{tag} hangul=\"{v}\" latin=\"{v}\" hanja=\"{v}\" japanese=\"{v}\" other=\"{v}\" symbol=\"{v}\" user=\"{v}\"/>"));
            }
            if *italic {chars.push_str("<hh:italic/>");}
            if *bold {chars.push_str("<hh:bold/>");}
            chars.push_str("<hh:underline type=\"NONE\" shape=\"SOLID\" color=\"#000000\"/><hh:strikeout shape=\"NONE\" color=\"#000000\"/><hh:outline type=\"NONE\"/><hh:shadow type=\"NONE\" color=\"#C0C0C0\" offsetX=\"10\" offsetY=\"10\"/></hh:charPr>");
        }
        let mut paras=String::new();
        for (i,(align,p,bullet)) in self.paras.iter().enumerate() {
            let heading=match bullet {Some(NativeHeading::Bullet(id))=>format!("<hh:heading type=\"BULLET\" idRef=\"{id}\" level=\"0\"/>"),Some(NativeHeading::Number(id))=>format!("<hh:heading type=\"NUMBER\" idRef=\"{id}\" level=\"0\"/>"),None=>"<hh:heading type=\"NONE\" idRef=\"0\" level=\"0\"/>".into()};
            paras.push_str(&format!("<hh:paraPr id=\"{i}\" tabPrIDRef=\"0\" condense=\"0\" fontLineHeight=\"0\" snapToGrid=\"1\" suppressLineNumbers=\"0\" checked=\"0\" textDir=\"LTR\"><hh:align horizontal=\"{align}\" vertical=\"BASELINE\"/>{heading}<hh:breakSetting breakLatinWord=\"KEEP_WORD\" breakNonLatinWord=\"KEEP_WORD\" widowOrphan=\"0\" keepWithNext=\"0\" keepLines=\"0\" pageBreakBefore=\"0\" lineWrap=\"BREAK\"/><hh:autoSpacing eAsianEng=\"0\" eAsianNum=\"0\"/><hh:margin>"));
            for (name,value) in [("intent",p.indent as i64),("left",p.left as i64),("right",p.right as i64),("prev",p.space_before as i64),("next",p.space_after as i64)] {paras.push_str(&format!("<hc:{name} value=\"{value}\" unit=\"HWPUNIT\"/>"));}
            let (line_type,line_value)=crate::line_spacing::resolve(p.line_spacing_percent,p.line_spacing.as_ref()).expect("validated paragraph spacing");
            paras.push_str(&format!("</hh:margin><hh:lineSpacing type=\"{line_type}\" value=\"{line_value}\" unit=\"HWPUNIT\"/><hh:border borderFillIDRef=\"2\" offsetLeft=\"0\" offsetRight=\"0\" offsetTop=\"0\" offsetBottom=\"0\" connect=\"0\" ignoreMargin=\"0\"/></hh:paraPr>"));
            if p.keep_with_next || p.keep_lines {
                let start=paras.rfind("<hh:paraPr").unwrap_or(0);
                let tail=paras[start..].replace("keepWithNext=\"0\"",if p.keep_with_next {"keepWithNext=\"1\""}else{"keepWithNext=\"0\""}).replace("keepLines=\"0\"",if p.keep_lines {"keepLines=\"1\""}else{"keepLines=\"0\""});
                paras.replace_range(start..,&tail);
            }
        }
        let mut bullet_xml=String::new();
        if !self.numberings.is_empty() {
            bullet_xml.push_str(&format!("<hh:numberings itemCnt=\"{}\">",self.numberings.len()));
            for (i,(n,cid)) in self.numberings.iter().enumerate() {
                let id=i+1;let start=n.start;
                bullet_xml.push_str(&format!("<hh:numbering id=\"{id}\" start=\"0\"><hh:paraHead start=\"{start}\" level=\"1\" align=\"LEFT\" useInstWidth=\"1\" autoIndent=\"1\" widthAdjust=\"0\" textOffsetType=\"PERCENT\" textOffset=\"50\" numFormat=\"DIGIT\" charPrIDRef=\"{cid}\" checkable=\"0\">^1.</hh:paraHead></hh:numbering>"));
            }
            bullet_xml.push_str("</hh:numberings>");
        }
        if !self.bullets.is_empty() {
            bullet_xml.push_str(&format!("<hh:bullets itemCnt=\"{}\">",self.bullets.len()));
            for (index,(marker,cid)) in self.bullets.iter().enumerate() {
                let id=index+1;
                bullet_xml.push_str(&format!("<hh:bullet id=\"{id}\" char=\"{marker}\" useImage=\"0\"><hh:paraHead level=\"0\" align=\"LEFT\" useInstWidth=\"1\" autoIndent=\"1\" widthAdjust=\"0\" textOffsetType=\"PERCENT\" textOffset=\"50\" numFormat=\"DIGIT\" charPrIDRef=\"{cid}\" checkable=\"0\"/></hh:bullet>"));
            }
            bullet_xml.push_str("</hh:bullets>");
        }
        let mut fonts=String::new();
        for lang in ["HANGUL","LATIN","HANJA","JAPANESE","OTHER","SYMBOL","USER"] {
            fonts.push_str(&format!("<hh:fontface lang=\"{lang}\" fontCnt=\"{}\">",self.fonts.len()));
            for (id,face) in self.fonts.iter().enumerate() {fonts.push_str(&format!("<hh:font id=\"{id}\" face=\"{}\" type=\"TTF\" isEmbedded=\"0\"/>",escape_xml(face).replace('"',"&quot;")));}
            fonts.push_str("</hh:fontface>");
        }
        let mut borders=String::new();
        for side in ["left","right","top","bottom"] {borders.push_str(&format!("<hh:{side}Border type=\"SOLID\" width=\"0.12 mm\" color=\"#000000\"/>"));}
        // Retain id 1 for table grids; plain character/paragraph defaults must
        // not inherit its visible frame (confirmed by native viewer A/B).
        let plain_sides=borders.replace("type=\"SOLID\"","type=\"NONE\"");
        let plain_border=format!("<hh:borderFill id=\"2\" threeD=\"0\" shadow=\"0\" centerLine=\"NONE\" breakCellSeparateLine=\"0\"><hh:slash type=\"NONE\" Crooked=\"0\" isCounter=\"0\"/><hh:backSlash type=\"NONE\" Crooked=\"0\" isCounter=\"0\"/>{plain_sides}<hh:diagonal type=\"NONE\" width=\"0.1 mm\" color=\"#000000\"/></hh:borderFill>");
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><hh:head xmlns:hh=\"{HH}\" xmlns:hc=\"{HC}\" version=\"1.4\" secCnt=\"1\"><hh:beginNum page=\"1\" footnote=\"1\" endnote=\"1\" pic=\"1\" tbl=\"1\" equation=\"1\"/><hh:refList><hh:fontfaces itemCnt=\"7\">{fonts}</hh:fontfaces><hh:borderFills itemCnt=\"2\"><hh:borderFill id=\"1\" threeD=\"0\" shadow=\"0\" centerLine=\"NONE\" breakCellSeparateLine=\"0\"><hh:slash type=\"NONE\" Crooked=\"0\" isCounter=\"0\"/><hh:backSlash type=\"NONE\" Crooked=\"0\" isCounter=\"0\"/>{borders}<hh:diagonal type=\"NONE\" width=\"0.1 mm\" color=\"#000000\"/></hh:borderFill>{plain_border}</hh:borderFills><hh:charProperties itemCnt=\"{}\">{chars}</hh:charProperties><hh:tabProperties itemCnt=\"1\"><hh:tabPr id=\"0\" autoTabLeft=\"0\" autoTabRight=\"0\"/></hh:tabProperties>{bullet_xml}<hh:paraProperties itemCnt=\"{}\">{paras}</hh:paraProperties><hh:styles itemCnt=\"1\"><hh:style id=\"0\" type=\"PARA\" name=\"Normal\" engName=\"Normal\" paraPrIDRef=\"0\" charPrIDRef=\"0\" nextStyleIDRef=\"0\" langID=\"1042\" lockForm=\"0\"/></hh:styles></hh:refList></hh:head>",self.chars.len(),self.paras.len())
    }
}
/// New editable package from a semantic specification → prepared bytes (no I/O).
pub fn create_bytes(spec:CreateSpec)->Result<Prepared> {
    let p=spec.page;
    if p.width==0 || p.height==0 || p.width>1000000 || p.height>1000000 || p.left.saturating_add(p.right)>=p.width || p.top.saturating_add(p.bottom)>=p.height {return Err("invalid_page_geometry");}
    if spec.blocks.is_empty() || spec.blocks.len()>1000 {return Err("invalid_blocks");}
    let mut b=Builder{chars:vec![(1000,false,"#000000".into(),false,[0;7])],fonts:vec![font()],paras:vec![("LEFT".into(),ParaProps::default(),None)],bullets:Vec::new(),numberings:Vec::new(),next_id:1,media:Vec::new(),cell_appearances:Vec::new()};
    let mut body=b.blocks(spec.blocks,p.width-p.left-p.right,0)?;
    let setup=format!("<hp:secPr id=\"\" textDirection=\"HORIZONTAL\" spaceColumns=\"1134\" tabStop=\"8000\" tabStopVal=\"4000\" tabStopUnit=\"HWPUNIT\" outlineShapeIDRef=\"0\" memoShapeIDRef=\"0\" textVerticalWidthHead=\"0\" masterPageCnt=\"0\"><hp:grid lineGrid=\"0\" charGrid=\"0\" wonggojiFormat=\"0\"/><hp:startNum pageStartsOn=\"BOTH\" page=\"0\" pic=\"0\" tbl=\"0\" equation=\"0\"/><hp:visibility hideFirstHeader=\"0\" hideFirstFooter=\"0\" hideFirstMasterPage=\"0\" border=\"SHOW_ALL\" fill=\"SHOW_ALL\" hideFirstPageNum=\"0\" hideFirstEmptyLine=\"0\" showLineNumber=\"0\"/><hp:pagePr landscape=\"WIDELY\" width=\"{}\" height=\"{}\" gutterType=\"LEFT_ONLY\"><hp:margin header=\"0\" footer=\"0\" gutter=\"0\" left=\"{}\" right=\"{}\" top=\"{}\" bottom=\"{}\"/></hp:pagePr></hp:secPr><hp:ctrl><hp:colPr id=\"\" type=\"NEWSPAPER\" layout=\"LEFT\" colCount=\"1\" sameSz=\"1\" sameGap=\"0\"/></hp:ctrl>",p.width,p.height,p.left,p.right,p.top,p.bottom);

    // Separate control run: first text remains independently formatable.
    let at=body.find("<hp:run").ok_or("invalid_blocks")?;
    // Native viewer A/B: NARROWLY rotates the supplied physical page dimensions.
    // Width/height already express the requested orientation; keep WIDELY.
    body.insert_str(at,&format!("<hp:run charPrIDRef=\"0\">{setup}</hp:run>"));
    let section=format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><hs:sec xmlns:hs=\"http://www.hancom.co.kr/hwpml/2011/section\" xmlns:hp=\"{HP}\" xmlns:hc=\"{HC}\">{body}</hs:sec>");
    let mut parts=vec![("mimetype",b"application/hwp+zip".to_vec()),
        ("version.xml",b"<?xml version=\"1.0\"?><hv:HCFVersion xmlns:hv=\"http://www.hancom.co.kr/hwpml/2011/version\" targetApplication=\"WORDPROCESSOR\" major=\"5\" minor=\"1\" micro=\"0\" buildNumber=\"0\" os=\"1\" xmlVersion=\"1.4\" application=\"Open-hancom\" appVersion=\"0.1.0\"/>".to_vec()),
        ("settings.xml",b"<ha:HWPApplicationSetting xmlns:ha=\"http://www.hancom.co.kr/hwpml/2011/app\"/>".to_vec()),
        ("META-INF/container.xml",b"<ocf:container xmlns:ocf=\"urn:oasis:names:tc:opendocument:xmlns:container\"><ocf:rootfiles><ocf:rootfile full-path=\"Contents/content.hpf\" media-type=\"application/hwpml-package+xml\"/></ocf:rootfiles></ocf:container>".to_vec()),
        ("META-INF/manifest.xml",b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"/>".to_vec()),
        ("Contents/content.hpf",b"<opf:package xmlns:opf=\"http://www.idpf.org/2007/opf/\" version=\"2.0\" id=\"document\" unique-identifier=\"document\"><opf:metadata/><opf:manifest><opf:item id=\"header\" href=\"Contents/header.xml\" media-type=\"application/xml\"/><opf:item id=\"section0\" href=\"Contents/section0.xml\" media-type=\"application/xml\"/></opf:manifest><opf:spine><opf:itemref idref=\"header\" linear=\"yes\"/><opf:itemref idref=\"section0\" linear=\"yes\"/></opf:spine></opf:package>".to_vec()),
        ("Contents/header.xml",{
            let extras=b.cell_appearances.iter().enumerate().map(|(i,a)|a.xml(i+3)).collect::<String>();
            b.header().replacen("<hh:borderFills itemCnt=\"2\">",&format!("<hh:borderFills itemCnt=\"{}\">",2+b.cell_appearances.len()),1).replacen("</hh:borderFills>",&format!("{extras}</hh:borderFills>"),1).into_bytes()
        }),("Contents/section0.xml",section.into_bytes())];
    let provenance=serde_json::json!({"engine":"open-hancom-core","version":"0.1.0","origin":"new semantic specification","native_validated":false,"full_schema_validated":false});
    parts.push(("META-INF/open-hancom.json",provenance.to_string().into_bytes()));
    let mut media_items=String::new();
    for (i,data) in b.media.iter().enumerate() {
        let id=i+1;
        let (ext,mime)=crate::inline::image_kind(data);
        media_items.push_str(&format!("<opf:item id=\"image{id}\" href=\"BinData/image{id}.{ext}\" media-type=\"{mime}\" isEmbeded=\"1\"/>"));
        // Part names are owned for the lifetime of ZIP writing below.
        let _=data;
    }
    let mut owned_parts:Vec<(String,Vec<u8>)>=parts.into_iter().map(|(n,b)|(n.to_string(),b)).collect();
    if !media_items.is_empty() {
        let (_,hpf)=owned_parts.iter_mut().find(|(n,_)|n=="Contents/content.hpf").ok_or("missing_package_part")?;
        *hpf=String::from_utf8(hpf.clone()).map_err(|_|"invalid_xml")?.replace("</opf:manifest>",&format!("{media_items}</opf:manifest>")).into_bytes();
    }
    for (i,data) in b.media.into_iter().enumerate() {let (ext,_)=crate::inline::image_kind(&data);owned_parts.push((format!("BinData/image{}.{ext}",i+1),data));}
    let mut zip=zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name,data) in owned_parts {
        if name.ends_with(".xml") || name.ends_with(".hpf") {roxmltree::Document::parse(std::str::from_utf8(&data).map_err(|_| "invalid_xml")?).map_err(|_| "invalid_xml")?;}
        zip.start_file(name,zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).map_err(|_| "zip_write_error")?;
        zip.write_all(&data).map_err(|_| "zip_write_error")?;
    }
    let raw=zip.finish().map_err(|_| "zip_write_error")?.into_inner();
    let package=Package::from_raw(raw)?;
    let mut prepared=package.prepare_changed(BTreeMap::new(),false)?;
    prepared.receipt["native_validated"]=serde_json::json!(false);
    prepared.receipt["full_schema_validated"]=serde_json::json!(false);
    Ok(prepared)
}
