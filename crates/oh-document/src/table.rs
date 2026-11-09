//! Conservative native grid editor. No hidden content deletion or geometry guessing.
use crate::{Package,Result,HP,tag_end};
use crate::styles::{standalone,set_attr,attr,patch,HH};
use serde::Deserialize;
use std::collections::BTreeMap;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableSpec { revision:String,target:String,operations:Vec<Op> }
#[derive(Deserialize)]
#[serde(tag="op",rename_all="snake_case",deny_unknown_fields)]
enum Op {
    Merge {row:usize,col:usize,rows:usize,cols:usize},
    Split {row:usize,col:usize,column_widths:Vec<u32>,row_heights:Vec<u32>},
    InsertRow {index:usize,height:u32,column_widths:Option<Vec<u32>>},
    /// Copy an existing row (styles, borders, paragraph shapes) to `index`. The source row must
    /// have no row spans in or across it.
    CloneRow {source:usize,index:usize}, DeleteRow {index:usize,row_heights:Option<Vec<u32>>},
    InsertColumn {index:usize,width:u32,row_heights:Option<Vec<u32>>}, DeleteColumn {index:usize,column_widths:Option<Vec<u32>>},
    Resize {column_widths:Vec<u32>,row_heights:Vec<u32>},
    Cell {row:usize,col:usize,margins:Option<Margins>,vertical_align:Option<String>,border_fill:Option<u32>},
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Margins {left:u32,right:u32,top:u32,bottom:u32}
struct Cell {raw:String,r:usize,c:usize,rs:usize,cs:usize,w:u32,h:u32}
struct Grid {raw:String,rows:usize,cols:usize,cells:Vec<Cell>,next_id:u32}
pub(crate) fn validate_grid(raw:String)->Result<()> {Grid::parse(raw,0).map(|_|())}
// Feasibility of exact interval sizes with strictly positive integer tracks.
// Equality edges plus monotonic bounds; a positive cycle means contradictory
// dimensions even when each row/column has the same overall total.
fn consistent_tracks(count:usize,spans:impl Iterator<Item=(usize,usize,u32)>)->Result<()> {
    let mut edges:Vec<(usize,usize,i64)>=(0..count).map(|i|(i,i+1,1)).collect();
    for (a,b,size) in spans {edges.push((a,b,size as i64));edges.push((b,a,-(size as i64)));}
    let mut distance=vec![0i64;count+1];
    for iteration in 0..=count {
        let mut changed=false;
        for &(a,b,w) in &edges {if distance[b]<distance[a]+w {distance[b]=distance[a]+w;changed=true;}}
        if !changed {return Ok(());}
        if iteration==count {return Err("inconsistent_table_tracks");}
    }
    Ok(())
}
fn doc(xml:&str)->Result<roxmltree::Document<'_>> {roxmltree::Document::parse(xml).map_err(|_| "invalid_table_xml")}
fn number(n:roxmltree::Node,key:&str)->Result<usize> {n.attribute(key).ok_or("missing_table_geometry")?.parse().map_err(|_| "invalid_table_geometry")}
fn child<'a,'i>(n:roxmltree::Node<'a,'i>,name:&str)->Result<roxmltree::Node<'a,'i>> {
    let found:Vec<_>=n.children().filter(|n| n.has_tag_name((HP,name))).collect();
    if found.len()!=1 {return Err("unsupported_table_structure");} Ok(found[0])
}
fn update(xml:&str,tag:&str,key:&str,value:&str)->Result<String> {
    let d=doc(xml)?;
    let n=child(d.root_element(),tag)?;
    patch(xml,vec![(n.range(),set_attr(xml,n,key,value)?)])
}
impl Cell {
    fn sync(&mut self)->Result<()> {
        for (tag,key,value) in [("cellAddr","rowAddr",self.r.to_string()),("cellAddr","colAddr",self.c.to_string()),
            ("cellSpan","rowSpan",self.rs.to_string()),("cellSpan","colSpan",self.cs.to_string()),
            ("cellSz","width",self.w.to_string()),("cellSz","height",self.h.to_string())] {self.raw=update(&self.raw,tag,key,&value)?;}
        Ok(())
    }
    fn standard(&self)->Result<()> {
        let d=doc(&self.raw)?;
        let n=d.root_element();
        if n.attributes().any(|a| a.namespace().is_some() || !["name","header","hasMargin","protect","editable","dirty","borderFillIDRef"].contains(&a.name()))
            || n.children().any(|c| !(c.is_text() && c.text().unwrap_or("").trim().is_empty()) && !(c.is_element() && c.tag_name().namespace()==Some(HP) && ["subList","cellAddr","cellSpan","cellSz","cellMargin"].contains(&c.tag_name().name()))) {
            return Err("opaque_cell_requires_preservation");
        }
        let list=child(n,"subList")?;
        if list.attributes().any(|a| a.namespace().is_some() || !["id","textDirection","lineWrap","vertAlign","linkListIDRef","linkListNextIDRef","textWidth","textHeight","hasTextRef","hasNumRef"].contains(&a.name()))
            || ["id","linkListIDRef","linkListNextIDRef","hasTextRef","hasNumRef"].iter().any(|k| list.attribute(*k).is_some_and(|v| v!="" && v!="0"))
            || n.attribute("name").is_some_and(|v| !v.is_empty()) || n.attribute("protect").is_some_and(|v| v!="0") {
            return Err("opaque_or_linked_cell_requires_preservation");
        }
        for (tag,allowed) in [("cellAddr",&["rowAddr","colAddr"][..]),("cellSpan",&["rowSpan","colSpan"][..]),("cellSz",&["width","height"][..]),("cellMargin",&["left","right","top","bottom"][..])] {
            let node=child(n,tag)?;
            if node.attributes().any(|a| a.namespace().is_some() || !allowed.contains(&a.name())) || node.children().any(|n| !n.is_text() || !n.text().unwrap_or("").trim().is_empty()) {return Err("opaque_cell_geometry_requires_preservation");}
        }
        Ok(())
    }
    fn empty(&self)->Result<bool> {
        let d=doc(&self.raw)?;
        let list=child(d.root_element(),"subList")?;
        // A blank-looking control/object is still content. Never discard it.
        Ok(list.descendants().skip(1).all(|n| {
            if n.is_element() && n.tag_name().namespace()==Some(HP) {
                let allowed:&[&str]=match n.tag_name().name() {
                    "p"=> &["id","paraPrIDRef","styleIDRef","pageBreak","columnBreak","merged"],
                    "run"=> &["charPrIDRef"], "t"=> &[], _=>return false,
                };
                n.attributes().all(|a| a.namespace().is_none() && allowed.contains(&a.name()))
                    && !(n.has_tag_name((HP,"p")) && ["pageBreak","columnBreak","merged"].iter().any(|k|n.attribute(*k).is_some_and(|v|v!="0")))
            } else {n.is_text() && if n.parent().is_some_and(|p| p.has_tag_name((HP,"t"))) {n.text().unwrap_or("").is_empty()} else {n.text().unwrap_or("").trim().is_empty()}}
        }))
    }
}
impl Grid {
    fn parse(raw:String,next_id:u32)->Result<Self> {
        let d=doc(&raw)?;
        let tbl=d.root_element();
        let rows=number(tbl,"rowCnt")?; let cols=number(tbl,"colCnt")?;
        if rows==0 || cols==0 || rows>100 || cols>100 {return Err("invalid_table_grid");}
        let row_nodes:Vec<_>=tbl.children().filter(|n| n.has_tag_name((HP,"tr"))).collect();
        if row_nodes.len()!=rows {return Err("invalid_table_grid");}
        let mut cells=Vec::new();
        for (r,row) in row_nodes.iter().enumerate() {
            if row.attributes().len()!=0 || row.children().any(|n| !(n.is_text() && n.text().unwrap_or("").trim().is_empty()) && !n.has_tag_name((HP,"tc"))) {return Err("opaque_row_requires_preservation");}
            for cell in row.children().filter(|n| n.has_tag_name((HP,"tc"))) {
                let addr=child(cell,"cellAddr")?;let span=child(cell,"cellSpan")?;let size=child(cell,"cellSz")?;
                let c=Cell{raw:standalone(&raw,cell)?,r:number(addr,"rowAddr")?,c:number(addr,"colAddr")?,rs:number(span,"rowSpan")?,cs:number(span,"colSpan")?,
                    w:u32::try_from(number(size,"width")?).map_err(|_| "invalid_table_geometry")?,h:u32::try_from(number(size,"height")?).map_err(|_| "invalid_table_geometry")?};
                if c.r!=r {return Err("invalid_table_grid");} cells.push(c);
            }
        }
        let grid=Self{raw,rows,cols,cells,next_id};grid.validate()?;
        let d=doc(&grid.raw)?;let sz=child(d.root_element(),"sz")?;
        let width:u64=grid.cells.iter().filter(|c|c.r==0).map(|c|c.w as u64).sum();
        let height:u64=grid.cells.iter().filter(|c|c.c==0).map(|c|c.h as u64).sum();
        if number(sz,"width")? as u64!=width || number(sz,"height")? as u64!=height {return Err("inconsistent_table_geometry");}
        Ok(grid)
    }
    fn validate(&self)->Result<()> {
        if self.rows==0 || self.cols==0 || self.rows>100 || self.cols>100 {return Err("invalid_table_grid");}
        let mut occupied=vec![false;self.rows*self.cols];
        for c in &self.cells {
            if c.rs==0 || c.cs==0 || c.r.checked_add(c.rs).is_none_or(|v| v>self.rows) || c.c.checked_add(c.cs).is_none_or(|v| v>self.cols)
                || c.w==0 || c.h==0 || c.w>1000000 || c.h>10000000 {return Err("invalid_table_geometry");}
            let d=doc(&c.raw)?;
            let m=child(d.root_element(),"cellMargin")?;
            if number(m,"left")?.saturating_add(number(m,"right")?)>=c.w as usize || number(m,"top")?.saturating_add(number(m,"bottom")?)>=c.h as usize {return Err("invalid_cell_margins");}
            for nested in d.descendants().filter(|n|n.has_tag_name((HP,"tbl")) && n.ancestors().skip(1).find(|n|n.has_tag_name((HP,"tc")))==Some(d.root_element())) {
                if number(child(nested,"sz")?,"width")?.saturating_add(number(m,"left")?).saturating_add(number(m,"right")?)>c.w as usize {return Err("nested_table_width_exceeded");}
            }
            for r in c.r..c.r+c.rs {for col in c.c..c.c+c.cs {
                let slot=&mut occupied[r*self.cols+col];if *slot {return Err("overlapping_table_cells");}*slot=true;
            }}
        }
        if occupied.iter().any(|v| !v) {return Err("incomplete_table_grid");}
        let mut widths=vec![0u64;self.rows];let mut heights=vec![0u64;self.cols];
        for c in &self.cells {for row in c.r..c.r+c.rs {widths[row]+=c.w as u64;}for col in c.c..c.c+c.cs {heights[col]+=c.h as u64;}}
        if widths.iter().any(|w|*w!=widths[0]) || heights.iter().any(|h|*h!=heights[0]) {return Err("inconsistent_table_geometry");}
        consistent_tracks(self.cols,self.cells.iter().map(|c|(c.c,c.c+c.cs,c.w)))?;
        consistent_tracks(self.rows,self.cells.iter().map(|c|(c.r,c.r+c.rs,c.h)))?;
        Ok(())
    }
    fn tracks(&self,columns:bool,explicit:Option<Vec<u32>>)->Result<Vec<u32>> {
        let count=if columns {self.cols}else{self.rows};
        let intervals:Vec<_>=self.cells.iter().map(|c|if columns {(c.c,c.c+c.cs,c.w)}else{(c.r,c.r+c.rs,c.h)}).collect();
        let tracks=if let Some(v)=explicit {v}else{
            // Recover only uniquely connected exact boundary differences. Never
            // distribute a merged interval by guessing equal-width tracks.
            let mut positions=vec![None;count+1];positions[0]=Some(0i64);
            for _ in 0..=count {for &(a,b,size) in &intervals {
                if let Some(x)=positions[a] {positions[b]=Some(x+size as i64);}
                else if let Some(x)=positions[b] {positions[a]=Some(x-size as i64);}
            }}
            (0..count).map(|i| {
                let size=positions[i+1].ok_or("ambiguous_table_tracks")?-positions[i].ok_or("ambiguous_table_tracks")?;
                u32::try_from(size).map_err(|_|"invalid_table_geometry")
            }).collect::<Result<Vec<_>>>()?
        };
        if tracks.len()!=count || tracks.iter().any(|v|*v==0 || *v>if columns {1000000}else{10000000}) {return Err("invalid_table_geometry");}
        for (a,b,size) in intervals {if tracks[a..b].iter().map(|v|*v as u64).sum::<u64>()!=size as u64 {return Err("inconsistent_table_tracks");}}
        Ok(tracks)
    }
    fn blank(&mut self,r:usize,c:usize,w:u32,h:u32)->Result<Cell> {
        let id=self.next_id;self.next_id=self.next_id.checked_add(1).ok_or("excess_object_ids")?;
        let raw=format!("<hp:tc xmlns:hp=\"{HP}\" name=\"\" header=\"0\" hasMargin=\"1\" protect=\"0\" editable=\"0\" dirty=\"0\" borderFillIDRef=\"1\"><hp:subList id=\"\" textDirection=\"HORIZONTAL\" lineWrap=\"BREAK\" vertAlign=\"CENTER\" linkListIDRef=\"0\" linkListNextIDRef=\"0\" textWidth=\"0\" textHeight=\"0\" hasTextRef=\"0\" hasNumRef=\"0\"><hp:p id=\"{id}\" paraPrIDRef=\"0\" styleIDRef=\"0\" pageBreak=\"0\" columnBreak=\"0\" merged=\"0\"><hp:run charPrIDRef=\"0\"><hp:t/></hp:run></hp:p></hp:subList><hp:cellAddr colAddr=\"{c}\" rowAddr=\"{r}\"/><hp:cellSpan colSpan=\"1\" rowSpan=\"1\"/><hp:cellSz width=\"{w}\" height=\"{h}\"/><hp:cellMargin left=\"0\" right=\"0\" top=\"0\" bottom=\"0\"/></hp:tc>");
        Ok(Cell{raw,r,c,rs:1,cs:1,w,h})
    }
    fn delete_track(&mut self,index:usize,columns:bool,explicit:Option<Vec<u32>>)->Result<()> {
        let count=if columns {self.cols}else{self.rows};
        if count<=1 || index>=count {return Err(if columns {"invalid_column_deletion"}else{"invalid_row_deletion"});}
        let tracks=self.tracks(columns,explicit)?;
        let mut remove=Vec::new();
        for (i,c) in self.cells.iter_mut().enumerate() {
            let (start,span)=if columns {(c.c,c.cs)}else{(c.r,c.rs)};
            if start<=index && index<start+span {
                if span==1 {c.standard()?;if !c.empty()? {return Err("would_delete_content");}remove.push(i);}
                else if columns {c.cs-=1;c.w=c.w.checked_sub(tracks[index]).ok_or("invalid_table_geometry")?;}
                else {c.rs-=1;c.h=c.h.checked_sub(tracks[index]).ok_or("invalid_table_geometry")?;}
            }
            if start>index {if columns {c.c-=1;}else{c.r-=1;}}
        }
        for i in remove.into_iter().rev() {self.cells.remove(i);}
        if columns {self.cols-=1;}else{self.rows-=1;}
        Ok(())
    }
    fn apply(&mut self,op:Op,border_ids:&[u32])->Result<()> {
        match op {
            Op::Merge{row,col,rows,cols}=> {
                if rows==0 || cols==0 || row.checked_add(rows).is_none_or(|v|v>self.rows) || col.checked_add(cols).is_none_or(|v|v>self.cols) {return Err("invalid_merge_rectangle");}
                let intersects=|c:&Cell|c.r<row+rows && c.r+c.rs>row && c.c<col+cols && c.c+c.cs>col;
                if self.cells.iter().any(|c|intersects(c) && (c.r<row || c.c<col || c.r+c.rs>row+rows || c.c+c.cs>col+cols)) {return Err("merge_crosses_existing_span");}
                let indices:Vec<_>=self.cells.iter().enumerate().filter(|(_,c)|intersects(c)).map(|(i,_)|i).collect();
                let width:u32=indices.iter().map(|i|&self.cells[*i]).filter(|c|c.r==row).map(|c|c.w).sum();
                let height:u32=indices.iter().map(|i|&self.cells[*i]).filter(|c|c.c==col).map(|c|c.h).sum();
                let mut paragraphs=String::new();
                let mut order=indices.clone();order.sort_by_key(|i| (self.cells[*i].r,self.cells[*i].c));
                for i in &order {
                    let c=&self.cells[*i];c.standard()?;
                    let d=doc(&c.raw)?;let list=child(d.root_element(),"subList")?;
                    if list.children().any(|n| !(n.is_text() && n.text().unwrap_or("").trim().is_empty()) && !n.has_tag_name((HP,"p"))) {return Err("opaque_cell_requires_preservation");}
                    for p in list.children().filter(|n| n.is_element()) {paragraphs.push_str(&standalone(&c.raw,p)?);}
                }
                let primary=*order.first().ok_or("invalid_merge_rectangle")?;
                let c=&mut self.cells[primary];
                let d=doc(&c.raw)?;let list=child(d.root_element(),"subList")?;
                let raw=&c.raw[list.range()];let end=tag_end(raw).ok_or("invalid_table_xml")?;
                let close=raw.rfind("</").ok_or("unsupported_empty_cell")?;
                c.raw=patch(&c.raw,vec![(list.range(),format!("{}{paragraphs}{}",&raw[..end],&raw[close..]))])?;
                c.rs=rows;c.cs=cols;c.w=width;c.h=height;
                for i in indices.into_iter().rev() {if i!=primary {self.cells.remove(i);}}
            }
            Op::Split{row,col,column_widths,row_heights}=> {
                let i=self.cells.iter().position(|c| c.r==row && c.c==col).ok_or("unknown_cell")?;
                let old=&self.cells[i];old.standard()?;
                if column_widths.len()!=old.cs || row_heights.len()!=old.rs || column_widths.iter().any(|v| *v==0) || row_heights.iter().any(|v| *v==0)
                    || column_widths.iter().map(|v| *v as u64).sum::<u64>()!=old.w as u64 || row_heights.iter().map(|v| *v as u64).sum::<u64>()!=old.h as u64 {return Err("invalid_split_geometry");}
                for (dr,height) in row_heights.iter().enumerate() {for (dc,width) in column_widths.iter().enumerate() {
                    if dr==0 && dc==0 {continue;}let blank=self.blank(row+dr,col+dc,*width,*height)?;self.cells.push(blank);
                }}
                let c=&mut self.cells[i];c.rs=1;c.cs=1;c.w=column_widths[0];c.h=row_heights[0];
            }
            Op::InsertRow{index,height,column_widths}=> {
                if index>self.rows || height==0 || self.rows>=100 {return Err("invalid_row_insertion");}
                let widths=self.tracks(true,column_widths)?;
                let mut covered=vec![false;self.cols];
                for c in &mut self.cells {
                    if c.r<index && index<c.r+c.rs {c.rs+=1;c.h=c.h.checked_add(height).ok_or("invalid_table_geometry")?;for col in c.c..c.c+c.cs {covered[col]=true;}}
                    else if c.r>=index {c.r+=1;}
                }
                for (col,w) in widths.into_iter().enumerate() {if !covered[col] {let cell=self.blank(index,col,w,height)?;self.cells.push(cell);}}
                self.rows+=1;
            }
            Op::CloneRow{source,index}=> {
                if source>=self.rows || index>self.rows || self.rows>=100 {return Err("invalid_row_insertion");}
                if self.cells.iter().any(|c| (c.r<=source && source<c.r+c.rs && c.rs!=1) || (c.r<index && index<c.r+c.rs)) {return Err("clone_row_crosses_span");}
                let mut copies=Vec::new();
                let mut fresh=Vec::new();
                for c in self.cells.iter().filter(|c| c.r==source) {c.standard()?;fresh.push(c.raw.clone());}
                for (raw,c) in fresh.into_iter().zip(self.cells.iter().filter(|c| c.r==source).map(|c|(c.c,c.cs,c.w,c.h)).collect::<Vec<_>>()) {
                    // cloned paragraphs get new ids (no duplicate paragraph ids)
                    let mut out=String::new();let mut rest=raw.as_str();
                    while let Some(i)=rest.find("<hp:p ") {
                        out.push_str(&rest[..i]);let tag_end_at=i+rest[i..].find('>').ok_or("invalid_table_xml")?;
                        let tag=&rest[i..tag_end_at];
                        let tag=match tag.find(" id=\"") {Some(k)=>{let v=k+5;let e=v+tag[v..].find('"').ok_or("invalid_table_xml")?;let id=self.next_id;self.next_id=self.next_id.checked_add(1).ok_or("excess_object_ids")?;format!("{}{}{}",&tag[..v],id,&tag[e..])}None=>tag.to_string()};
                        out.push_str(&tag);rest=&rest[tag_end_at..];
                    }
                    out.push_str(rest);
                    copies.push(Cell{raw:out,r:index,c:c.0,rs:1,cs:c.1,w:c.2,h:c.3});
                }
                for c in &mut self.cells {if c.r>=index {c.r+=1;}}
                self.cells.extend(copies);
                self.rows+=1;
            }
            Op::InsertColumn{index,width,row_heights}=> {
                if index>self.cols || width==0 || self.cols>=100 {return Err("invalid_column_insertion");}
                let heights=self.tracks(false,row_heights)?;
                let mut covered=vec![false;self.rows];
                for c in &mut self.cells {
                    if c.c<index && index<c.c+c.cs {c.cs+=1;c.w=c.w.checked_add(width).ok_or("invalid_table_geometry")?;for row in c.r..c.r+c.rs {covered[row]=true;}}
                    else if c.c>=index {c.c+=1;}
                }
                for (row,h) in heights.into_iter().enumerate() {if !covered[row] {let cell=self.blank(row,index,width,h)?;self.cells.push(cell);}}
                self.cols+=1;
            }
            Op::DeleteRow{index,row_heights}=>self.delete_track(index,false,row_heights)?,
            Op::DeleteColumn{index,column_widths}=>self.delete_track(index,true,column_widths)?,
            Op::Resize{column_widths,row_heights}=> {
                if column_widths.len()!=self.cols || row_heights.len()!=self.rows {return Err("invalid_table_geometry");}
                if column_widths.iter().any(|v|*v==0 || *v>1000000) || row_heights.iter().any(|v|*v==0 || *v>10000000) {return Err("invalid_table_geometry");}
                for c in &mut self.cells {
                    c.w=u32::try_from(column_widths[c.c..c.c+c.cs].iter().map(|v|*v as u64).sum::<u64>()).map_err(|_|"invalid_table_geometry")?;
                    c.h=u32::try_from(row_heights[c.r..c.r+c.rs].iter().map(|v|*v as u64).sum::<u64>()).map_err(|_|"invalid_table_geometry")?;
                }
            }
            Op::Cell{row,col,margins,vertical_align,border_fill}=> {
                let c=self.cells.iter_mut().find(|c| c.r==row && c.c==col).ok_or("unknown_cell")?;
                if let Some(m)=margins {
                    if m.left.saturating_add(m.right)>=c.w || m.top.saturating_add(m.bottom)>=c.h {return Err("invalid_cell_margins");}
                    for (key,v) in [("left",m.left),("right",m.right),("top",m.top),("bottom",m.bottom)] {c.raw=update(&c.raw,"cellMargin",key,&v.to_string())?;}
                    c.raw=attr(&c.raw,"hasMargin","1")?;
                }
                if let Some(v)=vertical_align {if !["TOP","CENTER","BOTTOM"].contains(&v.as_str()) {return Err("invalid_vertical_alignment");} c.raw=update(&c.raw,"subList","vertAlign",&v)?;}
                if let Some(id)=border_fill {if !border_ids.contains(&id) {return Err("unresolved_border_reference");}c.raw=attr(&c.raw,"borderFillIDRef",&id.to_string())?;}
            }
        }
        self.validate()?;
        for cell in &mut self.cells {cell.sync()?;}
        Ok(())
    }
    fn render(mut self)->Result<String> {
        self.cells.sort_by_key(|c| (c.r,c.c));
        let mut rows=String::new();
        for row in 0..self.rows {rows.push_str(&format!("<hp:tr xmlns:hp=\"{HP}\">"));for c in self.cells.iter().filter(|c| c.r==row) {rows.push_str(&c.raw);}rows.push_str("</hp:tr>");}
        let d=doc(&self.raw)?;let root=d.root_element();
        let old_rows:Vec<_>=root.children().filter(|n| n.has_tag_name((HP,"tr"))).collect();
        let first=old_rows.first().ok_or("invalid_table_grid")?.range().start;
        let last=old_rows.last().ok_or("invalid_table_grid")?.range().end;
        if root.children().any(|n| n.range().start>=first && n.range().end<=last && !(n.is_text() && n.text().unwrap_or("").trim().is_empty()) && !n.has_tag_name((HP,"tr"))) {return Err("opaque_row_requires_preservation");}
        let mut out=patch(&self.raw,vec![(first..last,rows)])?;
        out=attr(&out,"rowCnt",&self.rows.to_string())?;out=attr(&out,"colCnt",&self.cols.to_string())?;
        // Overall geometry is inferred from exact covering intervals, not summed cell boxes.
        let mut widths=vec![0u64;self.rows];let mut heights=vec![0u64;self.cols];
        for c in &self.cells {for r in c.r..c.r+c.rs {widths[r]+=c.w as u64;}for col in c.c..c.c+c.cs {heights[col]+=c.h as u64;}}
        if widths.iter().any(|w| *w!=widths[0]) || heights.iter().any(|h| *h!=heights[0]) {return Err("inconsistent_table_geometry");}
        out=update(&out,"sz","width",&widths[0].to_string())?;out=update(&out,"sz","height",&heights[0].to_string())?;Ok(out)
    }
}
impl Package {
    /// CLI contract: after the edit every table of the section must pass the grid check.
    pub fn table_bytes(&self,spec:TableSpec)->Result<crate::Prepared> {self.table_edit(spec,false)}
    /// Card engine: re-admit only the edited table (and tables inside it). Untouched native
    /// tables may carry auto-grown geometry the grid editor would reject.
    pub fn table_bytes_scoped(&self,spec:TableSpec)->Result<crate::Prepared> {self.table_edit(spec,true)}
    fn table_edit(&self,spec:TableSpec,scoped:bool)->Result<crate::Prepared> {
        if spec.revision!=self.inspection.revision {return Err("stale_document");}
        if spec.operations.is_empty() || spec.operations.len()>100 {return Err("invalid_operations");}
        let obj=self.inspection.tables.iter().find(|n| n.id==spec.target).ok_or("unknown_target")?;
        let index=spec.target.rsplit_once("#tbl").ok_or("unknown_target")?.1.parse::<usize>().map_err(|_| "unknown_target")?;
        let xml=std::str::from_utf8(&self.parts.iter().find(|(n,_)| n==&obj.part).ok_or("unknown_target")?.1).map_err(|_| "unsupported_xml_encoding")?;
        let d=doc(xml)?;let table=d.descendants().filter(|n| n.has_tag_name((HP,"tbl"))).nth(index).ok_or("unknown_target")?;
        let header=std::str::from_utf8(&self.parts.iter().find(|(n,_)| n=="Contents/header.xml").ok_or("missing_header")?.1).map_err(|_| "unsupported_xml_encoding")?;
        let h=roxmltree::Document::parse(header).map_err(|_| "invalid_header_xml")?;
        let border_ids:Vec<u32>=h.descendants().filter(|n| n.has_tag_name((HH,"borderFill"))).filter_map(|n| n.attribute("id").and_then(|s| s.parse().ok())).collect();
        // New blank cells have explicit valid defaults. Never synthesize dangling references.
        for (tag,id) in [("charPr","0"),("paraPr","0"),("style","0"),("borderFill","1")] {
            if !h.descendants().any(|n| n.has_tag_name((HH,tag)) && n.attribute("id")==Some(id)) {return Err("missing_default_table_styles");}
        }
        let mut grid=Grid::parse(standalone(xml,table)?,self.next_object_id()?)?;
        if spec.operations.iter().any(|op|matches!(op,Op::InsertRow{..}|Op::InsertColumn{..}|Op::DeleteRow{..}|Op::DeleteColumn{..})) {
            if table.attribute("lock").is_some_and(|v|v!="0") || table.ancestors().any(|n|n.has_tag_name((HP,"tc")) && n.attribute("protect").is_some_and(|v|v!="0")) {return Err("protected_table_edit");}
            let size=child(table,"sz")?;
            if table.attribute("cellSpacing").is_some_and(|v|v!="0") || size.attribute("widthRelTo")!=Some("ABSOLUTE") || size.attribute("heightRelTo")!=Some("ABSOLUTE") {return Err("unsupported_track_geometry");}
        }
        for op in spec.operations {grid.apply(op,&border_ids)?;}
        let replacement=grid.render()?;let replacement_len=replacement.len();
        let mut changed=BTreeMap::new();
        let candidate=patch(xml,vec![(table.range(),replacement)])?;
        let parsed=doc(&candidate)?;
        let old_ids:std::collections::BTreeSet<_>=table.descendants().filter_map(|n|n.attribute("id")).collect();
        let remaining:std::collections::BTreeSet<_>=parsed.descendants().filter_map(|n|n.attribute("id")).collect();
        let removed:std::collections::BTreeSet<_>=old_ids.difference(&remaining).copied().collect();
        if !removed.is_empty() {
            // Do not orphan visible native or opaque ID references. Definition
            // references use separate registries and are not paragraph IDs.
            for (name,bytes) in &self.parts {if name.ends_with(".xml") || name.ends_with(".hpf") {
                let s=if name==&obj.part {candidate.as_str()}else{std::str::from_utf8(bytes).map_err(|_|"invalid_xml")?};
                let p=doc(s)?;
                if p.descendants().any(|n|n.attributes().any(|a|removed.contains(a.value()) &&
                    (a.namespace().is_some() || a.name().ends_with("IDRef")) &&
                    !["paraPrIDRef","styleIDRef","charPrIDRef","borderFillIDRef","binaryItemIDRef"].contains(&a.name()))) {return Err("would_orphan_object_reference");}
            }}
        }
        // Re-admit the edited table and tables inside it. Untouched sibling tables keep their
        // native geometry (Hancom files often carry auto-grown sizes the grid editor would reject).
        let start=table.range().start;let end=start+replacement_len;
        for n in parsed.descendants().filter(|n|n.has_tag_name((HP,"tbl")) && (!scoped || (n.range().start>=start && n.range().end<=end))) {validate_grid(standalone(&candidate,n)?)?;}
        for n in parsed.descendants().filter(|n|n.has_tag_name((HP,"tbl")) && !n.ancestors().skip(1).any(|a|a.has_tag_name((HP,"tbl")))) {
            let pages:Vec<_>=parsed.descendants().filter(|p|p.has_tag_name((HP,"pagePr"))).collect();
            if pages.len()!=1 {return Err("unsupported_page_geometry");}
            let m=child(pages[0],"margin")?;
            let body=number(pages[0],"width")?.checked_sub(number(m,"left")?.saturating_add(number(m,"right")?)).ok_or("unsupported_page_geometry")?;
            if number(child(n,"sz")?,"width")?>body {return Err("table_body_width_exceeded");}
        }
        changed.insert(obj.part.clone(),candidate.into_bytes());
        self.prepare_changed(changed,false)
    }
}
