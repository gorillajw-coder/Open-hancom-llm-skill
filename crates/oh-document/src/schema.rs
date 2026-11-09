//! JSON Schema discovery for the exact supported CLI subset.
use serde_json::{Value,json};
fn object(props:Value,required:&[&str])->Value {json!({"type":"object","properties":props,"required":required,"additionalProperties":false})}
fn array(item:Value,min:usize,max:usize)->Value {json!({"type":"array","items":item,"minItems":min,"maxItems":max})}
fn integer(min:i64,max:i64)->Value {json!({"type":"integer","minimum":min,"maximum":max})}
fn string()->Value {json!({"type":"string"})}
fn boolean()->Value {json!({"type":"boolean"})}
fn measure()->Value {integer(0,1000000)}
fn align()->Value {json!({"enum":["LEFT","RIGHT","CENTER","JUSTIFY","DISTRIBUTE","DISTRIBUTE_SPACE"]})}
fn color()->Value {json!({"type":"string","pattern":"^#[0-9A-Fa-f]{6}$"})}
fn widths()->Value {array(integer(1,1000000),1,100)}
fn heights()->Value {array(integer(1,10000000),1,100)}
fn spec(properties:Value,required:&[&str])->Value {
    let mut p=properties;
    p["revision"]=json!({"type":"string","pattern":"^[0-9a-f]{64}$"});p["target"]=string();
    let mut r=vec!["revision","target"];r.extend_from_slice(required);object(p,&r)
}
fn command(syntax:&str,schema:Value)->Value {json!({"syntax":syntax,"schema":schema})}
pub fn discovery()->Value {
    let replace=object(json!({"op":{"const":"replace_text"},"target":string(),"expected":string(),"text":string()}),&["op","target","expected","text"]);
    let apply=object(json!({"revision":{"type":"string","pattern":"^[0-9a-f]{64}$"},"operations":array(replace,1,1000)}),&["revision","operations"]);
    let font_refs=object(json!({"hangul":integer(0,9999),"latin":integer(0,9999),"hanja":integer(0,9999),"japanese":integer(0,9999),"other":integer(0,9999),"symbol":integer(0,9999),"user":integer(0,9999)}),&[]);
    let character=object(json!({"height":integer(1,100000),"bold":boolean(),"italic":boolean(),"font_refs":font_refs,"textColor":color()}),&[]);
    let formatting=spec(json!({"properties":character,"range":object(json!({"start":integer(0,2000000),"end":integer(0,2000000)}),&["start","end"])}),&["properties"]);
    let line_spacing=object(json!({"type":{"enum":["exact","minimum"]},"value":integer(1,1000000),"unit":{"const":"HWPUNIT"}}),&["type","value","unit"]);
    let mut para=object(json!({"align":align(),"indent":integer(-1000000,1000000),"left":measure(),"right":measure(),"space_before":measure(),"space_after":measure(),"keep_with_next":boolean(),"keep_lines":boolean(),"line_spacing_percent":integer(1,1000),"line_spacing":line_spacing}),&[]);
    para["not"]=json!({"required":["line_spacing","line_spacing_percent"]});
    let margins=object(json!({"left":measure(),"right":measure(),"top":measure(),"bottom":measure()}),&["left","right","top","bottom"]);
    let mut ops=Vec::new();
    for (op,fields,required) in [
        ("merge",json!({"row":integer(0,99),"col":integer(0,99),"rows":integer(1,100),"cols":integer(1,100)}),vec!["row","col","rows","cols"]),
        ("split",json!({"row":integer(0,99),"col":integer(0,99),"column_widths":widths(),"row_heights":heights()}),vec!["row","col","column_widths","row_heights"]),
        ("insert_row",json!({"index":integer(0,100),"height":integer(1,10000000),"column_widths":widths()}),vec!["index","height"]),
        ("insert_column",json!({"index":integer(0,100),"width":integer(1,1000000),"row_heights":heights()}),vec!["index","width"]),
        ("delete_row",json!({"index":integer(0,99),"row_heights":heights()}),vec!["index"]),
        ("delete_column",json!({"index":integer(0,99),"column_widths":widths()}),vec!["index"]),
        ("resize",json!({"column_widths":widths(),"row_heights":heights()}),vec!["column_widths","row_heights"]),
        ("cell",json!({"row":integer(0,99),"col":integer(0,99),"margins":margins,"vertical_align":{"enum":["TOP","CENTER","BOTTOM"]},"border_fill":integer(1,10000)}),vec!["row","col"]),
    ] {
        let mut props=fields;props["op"]=json!({"const":op});let mut r=vec!["op"];r.extend(required);ops.push(object(props,&r));
    }
    let table=spec(json!({"operations":array(json!({"oneOf":ops}),1,100)}),&["operations"]);
    let image=object(json!({"png_hex":{"type":"string","pattern":"^(?:[0-9A-Fa-f]{2})+$","maxLength":2097152},"width":integer(1,1000000),"height":integer(1,1000000)}),&["png_hex","width","height"]);
    let jpeg_image=object(json!({"format":{"const":"jpeg"},"jpeg_hex":image["properties"]["png_hex"].clone(),"width":integer(1,1000000),"height":integer(1,1000000)}),&["format","jpeg_hex","width","height"]);
    let create_image=json!({"oneOf":[image.clone(),jpeg_image]});
    let mut image_replace=spec(json!({"expected_binary_item_id":string(),"expected_sha256":{"type":"string","pattern":"^[0-9a-f]{64}$"},"png_hex":image["properties"]["png_hex"].clone(),"format":{"const":"jpeg"},"jpeg_hex":image["properties"]["png_hex"].clone()}),&["expected_binary_item_id","expected_sha256"]);
    image_replace["oneOf"]=json!([
        {"required":["png_hex"],"not":{"anyOf":[{"required":["format"]},{"required":["jpeg_hex"]}]}},
        {"required":["format","jpeg_hex"],"not":{"required":["png_hex"]}}
    ]);
    let url=json!({"type":"string","maxLength":4096,"pattern":"^https?://"});
    let font_name=json!({"type":"string","minLength":1,"maxLength":256});
    let font_families=object(json!({"hangul":font_name,"latin":font_name,"hanja":font_name,"japanese":font_name,"other":font_name,"symbol":font_name,"user":font_name}),&[]);
    let run=object(json!({"text":string(),"height":integer(1,100000),"bold":boolean(),"italic":boolean(),"font_family":font_name,"font_families":font_families,"color":color(),"image":create_image,"hyperlink":url}),&["text"]);
    let mut generation_para=para.clone();generation_para["properties"].as_object_mut().unwrap().remove("align");
    let marker_style=object(json!({"height":integer(1,100000),"color":color(),"font_family":font_name,"bold":boolean(),"italic":boolean()}),&["height","color","font_family","bold","italic"]);
    let list=object(json!({"kind":{"const":"bullet"},"marker":{"enum":["•","■"]},"marker_style":marker_style}),&["kind","marker"]);
    let numbering=object(json!({"group":{"type":"string","minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9_-]+$"},"start":integer(1,65535),"format":{"const":"decimal-dot"},"marker_style":marker_style}),&["group","start","format"]);
    let mut block_para=object(json!({"type":{"const":"paragraph"},"align":align(),"page_break":boolean(),"properties":generation_para,"list":list,"numbering":numbering,"runs":array(run.clone(),1,1000)}),&["type","runs"]);
    block_para["allOf"]=json!([
        {"not":{"required":["list","numbering"]}},
        {"if":{"required":["numbering"]},"then":{"properties":{
            "page_break":{"const":false},
            "properties":{"properties":{"keep_with_next":{"const":false},"keep_lines":{"const":false}}},
            "runs":{"items":{"not":{"anyOf":[{"required":["image"]},{"required":["hyperlink"]}]}}}
        }}}
    ]);
    let mut cell=object(json!({"text":string(),"runs":array(run,1,1000),"blocks":array(json!({"$ref":"#/$defs/block","allOf":[{"$ref":"#/$defs/cellBlock"}]}),1,1000),"rowspan":integer(1,100),"colspan":integer(1,100),"margins":margins,"vertical_align":{"enum":["TOP","CENTER","BOTTOM"]}}),&[]);
    let side=object(json!({"style":{"enum":["NONE","SOLID"]},"width":{"enum":crate::border::WIDTHS},"color":color()}),&["style","width","color"]);
    cell["properties"]["appearance"]=object(json!({"fill_color":color(),"borders":object(json!({"left":side,"right":side,"top":side,"bottom":side}),&[])}),&[]);
    cell["oneOf"]=json!([{"required":["text"]},{"required":["runs"]},{"required":["blocks"]}]);
    let block_table=object(json!({"type":{"const":"table"},"column_widths":widths(),"row_heights":heights(),"rows":array(array(cell,0,100),1,100)}),&["type","column_widths","row_heights","rows"]);
    let mut cell_para=block_para.clone();cell_para["allOf"].as_array_mut().unwrap().push(json!({"not":{"anyOf":[{"required":["numbering"]},{"required":["list"]}]}}));
    let cell_block=json!({"oneOf":[cell_para,block_table.clone()]});
    let block=json!({"oneOf":[block_para,block_table]});
    let mut create=object(json!({"page":object(json!({"width":integer(1,1000000),"height":integer(1,1000000),"left":measure(),"right":measure(),"top":measure(),"bottom":measure()}),&["width","height","left","right","top","bottom"]),"blocks":array(block.clone(),1,1000)}),&["blocks"]);
    create["$defs"]=json!({"block":block,"cellBlock":cell_block});
    json!({"engine":"open-hancom-core","version":"0.1.0","schema_dialect":"https://json-schema.org/draft/2020-12/schema",
        "commands":{
            "inspect":{"syntax":"inspect INPUT.hwpx","exposes_local_text":true,"paragraph_structure":"direct_run_count and indexed direct_runs (character ref, child tags, field begin/end IDs); descriptive, not blanket insertion permission"},
            "validate":{"syntax":"validate INPUT.hwpx","scope":"supported-structure diagnostics only; valid_supported_structure false is a report, not CLI execution failure"},
            "diff":{"syntax":"diff BEFORE.hwpx AFTER.hwpx","scope":"hash-only member payload changes; no text"},
            "apply":command("apply INPUT.hwpx --plan PLAN.json --output NEW.hwpx",apply),
            "image":command("image INPUT.hwpx --spec SPEC.json --output NEW.hwpx",spec(json!({"width":integer(1,1000000),"height":integer(1,1000000)}),&["width","height"])),
            "image-replace":command("image-replace INPUT.hwpx --spec SPEC.json --output NEW.hwpx",image_replace),
            "image-insert":command("image-insert INPUT.hwpx --spec SPEC.json --output NEW.hwpx",spec(json!({"expected_run_count":integer(1,999),"position":integer(0,999),"char_pr_id":integer(0,4294967295),"image":create_image}),&["expected_run_count","position","char_pr_id","image"])),
            "link":command("link INPUT.hwpx --spec SPEC.json --output NEW.hwpx",spec(json!({"url":url}),&["url"])),
            "format":command("format INPUT.hwpx --spec SPEC.json --output NEW.hwpx",formatting),
            "paragraph":command("paragraph INPUT.hwpx --spec SPEC.json --output NEW.hwpx",spec(json!({"properties":para}),&["properties"])),
            "table":command("table INPUT.hwpx --spec SPEC.json --output NEW.hwpx",table),
            "style":command("style INPUT.hwpx --spec SPEC.json --output NEW.hwpx",spec(json!({"name":string(),"paragraph_definition":integer(0,9999),"character_definition":integer(0,9999)}),&["name"])),
            "border":command("border INPUT.hwpx --spec SPEC.json --output NEW.hwpx",spec(json!({"properties":object(json!({"border_color":color(),"border_width":{"enum":["0.1 mm","0.12 mm","0.15 mm","0.2 mm","0.25 mm","0.3 mm","0.4 mm","0.5 mm","0.6 mm","0.7 mm","1.0 mm","1.5 mm","2.0 mm","3.0 mm","4.0 mm","5.0 mm"]},"fill_color":color()}),&[])}),&["properties"])),
            "create":command("create --spec SPEC.json --output NEW.hwpx",create)},
        "semantics":{"range":"half-open Unicode scalar offsets, not UTF-8 bytes or grapheme clusters","measure":"HWPUNIT: 100 per point","atomicity":"one command, all-or-nothing; output path never overwritten","table_merge":"row-major paragraphs; top-left cell formatting retained; opaque/linked cells reject","table_split":"original content stays top-left; supplied dimensions must sum exactly","table_delete":"empty supported cells only","style":"native definition clone/reuse; no implicit global restyling"},
        "unsupported":["native_layout","full_schema_certification","raster_screen_ocr","non_PNG_media","external_image_fetch","arbitrary_picture_editing","image_insertion_into_existing_packages","global_named_style_redefinition","arbitrary_control_run_splitting","merged_grid_row_column_insertion_deletion","ZIP64_editing"],
        "unsupported_scope":"Legacy identifiers denote unrestricted operation families, not blanket rejection of the explicitly admitted bounded variants in capabilities/commands. Retained for discovery-client compatibility.",
        "capabilities":{
            "bounded_jpeg":{"supported_subset":true,"entrypoints":["create","inspect","validate","image-replace","image-insert"],"descriptor":"format:jpeg + jpeg_hex + width + height; exclusive with unchanged legacy png_hex descriptor","decoder":"zune-jpeg-0.5.15+open-hancom-completeness-v1","subset_version":crate::jpeg::SUBSET,"supported_sampling":["4:4:4","4:2:2","4:2:0"],"subset":"JFIF1.00..1.02;8bit baseline YCbCr 4:4:4/4:2:2(horizontal)/4:2:0;single interleaved scan;no thumbnails;equal nonzero density axes","limits":{"input_bytes":1048576,"pixel_axis":4096,"pixels":4000000,"decoded_rgb_bytes":12000000,"working_buffer_bytes":1048576},"pixel_geometry":"decoded_image pixel_width/pixel_height;separate from native HWPUNIT width/height","resource":"native pic/hc:img binaryItemIDRef -> actual embedded OPF image/jpeg -> safe BinData/imageN.jpg; exact original bytes and SHA256","binary_reencoded":false,"native_render_verified":false,"jpeg_replace_insert":true,"editing_contract":"scoped-jpeg-media-edit-v1; replacement top-level format:jpeg+jpeg_hex preserves native geometry and requires existing image/jpeg; insertion uses create image descriptor at admitted direct-run boundary; no PNG/JPEG conversion; COW retains all old resources","unsupported":["EXIF/orientation","ICC/MPO/XMP/Adobe/CMYK","progressive/lossless/arithmetic","sampling other than conventional Y=11/21/22,Cb=Cr=11","metadata/extensions/comments","restart/multiscan","non-square pixels","HTML JPEG import"]},
            "native_list_marker_style":{"supported_subset":true,"entrypoints":["create"],"property":"list.marker_style or numbering.marker_style","required":["height","color","font_family","bold","italic"],"height_unit":"HWPUNIT:100 per point;1..100000","color":"direct #RRGGBB","font_family":"explicit uniform family across seven script registries;not inherited from body","omission_char_pr_id":0,"binding":"saved paraPr/heading/list-definition/paraHead/charPr/fontRefs and actual font definitions;current revision only","identity":"bullet marker + resolved character style;numbering explicit group + consistent start/format/resolved style","font_availability_verified":false,"native_visual_verified":false,"docx_marker_rpr_import":false,"unsupported":["raw XML or IDs","partial marker style","per-script marker families","theme/inherited source marker style","existing list assignment or definition mutation"]},
            "native_decimal_numbering":{"supported_subset":true,"entrypoints":["create"],"paragraph_property":"numbering:{group,start,format:decimal-dot}; mutually exclusive with list","heading_level":0,"para_head_level":1,"definition_start":0,"level_start":"explicit 1..65535; declaration, not observed current number","format_text":"^1.","group_identity":"same explicit group reuses definition only with identical settings; different groups never merged; group is create-local, saved definition ID is persistent identity","counter_state_computed":false,"containers":"BODY nonblank text runs only; no keep/page-break/image/link numbered paragraph; independent ordinary table/nested/link/PNG blocks supported","character_style":"omission: explicit default charPr0; opt-in marker_style: independent typed height/color/family/bold/italic through native_list_marker_style","inspect":"revision-bound native_list from actual saved paragraph/paraPr/NUMBER/numbering/paraHead/charPr references, raw attrs and format text","unsupported":"restart/current calculation, multilevel/mixed formats, DOCX/HTML list import, existing-list assignment/catalog mutation, full XSD, native continuity or visual certification"},
            "native_paragraph_bullets":{"supported_subset":true,"entrypoints":["create"],"markers":["•","■"],"level":0,"decimal_numbering":false,"marker_character_style":"omission: default charPr0 (10pt black/default family); opt-in typed marker_style through native_list_marker_style, independent of body runs","containers":"body text paragraphs only; no page-break, keep, image or hyperlink list paragraph; separate ordinary rich/table blocks remain supported","inspect":"revision-bound native_list binding from saved header and actual paragraph paraPr reference","unsupported":"DOCX/HTML list import, numbered/restart/multilevel lists, existing-list assignment or definition mutation, checkbox/image bullets, rendering and font availability"},
            "absolute_paragraph_line_spacing":{"supported_subset":true,"entrypoints":["create","paragraph"],"unit":"HWPUNIT: 100 per point","exact":"FIXED native top-to-top advance, not Word line-box/clipping parity","minimum":"AT_LEAST native minimum top-to-top advance; native metrics differ and mixed-run baseline gaps are not guaranteed by this measure","bounds":"1..1000000 HWPUNIT; mutually exclusive with percent; omit create properties for legacy 160%; edit via explicit percent to clear absolute intent","layout":"no source grid/metrics/pagination or renderer equivalence"},
            "merged_track_insertion_deletion":{"supported_subset":true,"insertion":"strict interior expands span; boundary shifts anchors; uncovered slots are new empty cells","deletion":"surviving spans shrink/reanchor with all content; entirely removed cells must be supported truly empty","geometry":"explicit new size; existing tracks uniquely recovered from exact equations or supplied and checked; ambiguity rejects","unsupported":"opaque rows, unknown geometry semantics, content removal, unresolved dimensions"},
            "scoped_png_replacement":{"supported_subset":true,"model":"exact generated picture model; strict expected binary ID/hash; always COW, originals retained"},
            "scoped_png_insertion":{"supported_subset":true,"model":"supported direct-run paragraph at field-exterior boundary, explicit existing character definition; body fit checked; no text-run splitting or opaque/cached-layout paragraph normalization"}},
        "errors":{"exit_code":2,"response":{"error":"stable_code","written":false}},
        "bounds":{"input_bytes":67108864,"expanded_bytes":134217728,"entry_bytes":33554432,"entries":4096,"spec_bytes":2097152}})
}
