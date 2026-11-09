//! Explicit occupancy table generation and bounded recursive block cells.
use super::*;
impl Builder {
    pub(super) fn blocks(&mut self,blocks:Vec<Block>,available:u32,depth:usize)->Result<String> {
        if depth>8 || blocks.is_empty() || blocks.len()>1000 {return Err("invalid_or_excess_nested_blocks");}
        let mut out=String::new();
        for block in blocks {
            if self.next_id>100000 {return Err("excess_object_ids");}
            match block {
                Block::Paragraph{runs,align,page_break,properties,list,numbering}=> {
                    if (list.is_some() || numbering.is_some()) && (depth!=0 || page_break) {return Err("unsupported_native_list_container_or_page_break");}
                    let p=self.para_with(runs,&align,properties,list,numbering)?;
                    out.push_str(&if page_break {p.replacen("pageBreak=\"0\"","pageBreak=\"1\"",1)}else{p});
                }
                Block::Table{column_widths,row_heights,rows}=> {
                    let nr=rows.len();let nc=column_widths.len();
                    if nr==0 || nr>100 || nc==0 || nc>100 || row_heights.len()!=nr {return Err("invalid_table_grid");}
                    let width:u64=column_widths.iter().map(|v|*v as u64).sum();let height:u64=row_heights.iter().map(|v|*v as u64).sum();
                    if column_widths.iter().any(|v|*v<=1020) || row_heights.iter().any(|v|*v<=282) || width>available as u64 || height>10000000 {return Err("invalid_table_geometry");}
                    let id=self.id();
                    let mut table=format!("<hp:tbl id=\"{id}\" zOrder=\"0\" numberingType=\"TABLE\" textWrap=\"TOP_AND_BOTTOM\" textFlow=\"BOTH_SIDES\" lock=\"0\" dropcapstyle=\"None\" pageBreak=\"CELL\" repeatHeader=\"1\" rowCnt=\"{nr}\" colCnt=\"{nc}\" cellSpacing=\"0\" borderFillIDRef=\"1\" noAdjust=\"0\"><hp:sz width=\"{width}\" widthRelTo=\"ABSOLUTE\" height=\"{height}\" heightRelTo=\"ABSOLUTE\" protect=\"0\"/><hp:pos treatAsChar=\"1\" affectLSpacing=\"0\" flowWithText=\"1\" allowOverlap=\"0\" holdAnchorAndSO=\"0\" vertRelTo=\"PARA\" horzRelTo=\"COLUMN\" vertAlign=\"TOP\" horzAlign=\"LEFT\" vertOffset=\"0\" horzOffset=\"0\"/><hp:outMargin left=\"0\" right=\"0\" top=\"0\" bottom=\"0\"/><hp:inMargin left=\"510\" right=\"510\" top=\"141\" bottom=\"141\"/>");
                    let mut occupied=vec![false;nr*nc];
                    for (r,row) in rows.into_iter().enumerate() {
                        table.push_str("<hp:tr>");let mut c=0;
                        for cell in row {
                            let border_id=if let Some(a)=cell.appearance {self.cell_appearance(a)?}else{1};
                            while c<nc && occupied[r*nc+c] {c+=1;}
                            let (rs,cs)=(cell.rowspan,cell.colspan);
                            if rs==0 || cs==0 || r.checked_add(rs).is_none_or(|v|v>nr) || c.checked_add(cs).is_none_or(|v|v>nc) {return Err("invalid_table_span");}
                            for rr in r..r+rs {for cc in c..c+cs {if std::mem::replace(&mut occupied[rr*nc+cc],true) {return Err("overlapping_table_cells");}}}
                            let w:u32=column_widths[c..c+cs].iter().sum();let h:u32=row_heights[r..r+rs].iter().sum();
                            let m=cell.margins;
                            if m.left.saturating_add(m.right)>=w || m.top.saturating_add(m.bottom)>=h || !["TOP","CENTER","BOTTOM"].contains(&cell.vertical_align.as_str()) {return Err("invalid_cell_margins_or_alignment");}
                            let content=match (cell.text,cell.runs,cell.blocks) {
                                (Some(text),None,None)=>self.para(vec![Run{text,height:1000,bold:false,italic:false,font_family:font(),font_families:BTreeMap::new(),color:black(),image:None,hyperlink:None}],"LEFT")?,
                                (None,Some(runs),None)=>self.para(runs,"LEFT")?,
                                (None,None,Some(blocks))=>self.blocks(blocks,w-m.left-m.right,depth+1)?,
                                _=>return Err("invalid_cell_content"),
                            };
                            table.push_str(&format!("<hp:tc name=\"\" header=\"{}\" hasMargin=\"1\" protect=\"0\" editable=\"0\" dirty=\"0\" borderFillIDRef=\"{border_id}\"><hp:subList id=\"\" textDirection=\"HORIZONTAL\" lineWrap=\"BREAK\" vertAlign=\"{}\" linkListIDRef=\"0\" linkListNextIDRef=\"0\" textWidth=\"0\" textHeight=\"0\" hasTextRef=\"0\" hasNumRef=\"0\">{content}</hp:subList><hp:cellAddr colAddr=\"{c}\" rowAddr=\"{r}\"/><hp:cellSpan colSpan=\"{cs}\" rowSpan=\"{rs}\"/><hp:cellSz width=\"{w}\" height=\"{h}\"/><hp:cellMargin left=\"{}\" right=\"{}\" top=\"{}\" bottom=\"{}\"/></hp:tc>",if r==0{1}else{0},cell.vertical_align,m.left,m.right,m.top,m.bottom));
                            c+=cs;
                        }
                        table.push_str("</hp:tr>");
                    }
                    if occupied.iter().any(|v|!*v) {return Err("incomplete_table_grid");}
                    table.push_str("</hp:tbl><hp:t/>");let pid=self.id();
                    out.push_str(&format!("<hp:p id=\"{pid}\" paraPrIDRef=\"0\" styleIDRef=\"0\" pageBreak=\"0\" columnBreak=\"0\" merged=\"0\"><hp:run charPrIDRef=\"0\">{table}</hp:run></hp:p>"));
                }
            }
            if out.len()>crate::MAX_ENTRY as usize {return Err("entry_too_large");}
        }
        Ok(out)
    }
}
