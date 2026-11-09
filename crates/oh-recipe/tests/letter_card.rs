//! official-letter frame + client-report-letter card (main#4, D12).
//! Expected texts are written by hand from the frame's layout rules, not with the engine.
use oh_document::Package;
use oh_recipe::{finalize, Card, Defaults, Engine, Frame, Plan};
use serde_json::{json, Value};

const FRAME: &str = include_str!("../../../library/frames/official-letter.toml");
const CARD: &str = include_str!("../../../library/cards/client-report-letter.toml");
const DEFAULTS: &str = include_str!("../../../library/defaults.toml");
const TEMPLATE: &[u8] = include_bytes!("../../../library/templates/official-letter.hwpx");

fn engine() -> Engine {
    let c = Card::from_toml(CARD).unwrap().with_frames(&[Frame::from_toml(FRAME).unwrap()]).unwrap();
    Engine::new(Defaults::from_toml(DEFAULTS).unwrap(), vec![(c, TEMPLATE.to_vec())]).unwrap()
}
fn u(v: &str) -> Value {
    json!({"value": v, "source": "사용자"})
}
fn llm(v: &str) -> Value {
    json!({"value": v, "source": "llm"})
}
fn plan(slots: Value) -> Plan {
    oh_document::parse_contract(json!({"documents": [{"card": "client-report-letter", "slots": slots}]}).to_string().as_bytes(), "invalid_plan").unwrap()
}
fn cell(bytes: &[u8], row: usize) -> String {
    Package::from_bytes(bytes.to_vec()).unwrap().table_cells().unwrap().into_iter().find(|c| c.table == 0 && c.row == row && c.col == if (1..=4).contains(&row) { 1 } else { 0 }).unwrap().text
}
fn full() -> Value {
    json!({
        "doc_date": u("2026-03-02"), "doc_no": u("개좋소-2026-031"),
        "recipient": llm("김밥천국공사 사장"), "cc": llm("사업관리팀장"),
        "doc_title": llm("저녁메뉴 장보기 3월 진도 보고"),
        "body": llm("1. 관련: 저녁메뉴 장보기 용역 계약서 제7조\n위 호와 관련하여 3월 진도를 아래와 같이 보고합니다."),
        "items": llm("가. 공정: 계획 대비 정상\n인력: 4명 투입\n- 다음 달 계획: 현장 조사"),
        "attachments": llm("월간 진도 보고서 1부.\n사진 대지 1부."),
        "phone": u("000-0000-0000"), "email": u("kim@example.com"), "author_name": u("김철수"),
    })
}

#[test]
fn letter_layout_numbering_and_end_mark() {
    let e = engine();
    let r = e.plan_check(&plan(full()));
    assert!(r.ok && r.finalizable, "{:?} {:?}", r.errors, r.blockers);
    let d = e.draft(&plan(full())).unwrap();
    let b = &d.documents[0].bytes;
    assert_eq!(cell(b, 0), "개좋소"); // default our_company (D6-2)
    assert_eq!(cell(b, 1), "김밥천국공사 사장");
    assert_eq!(cell(b, 2), ""); // 경유 not given
    assert_eq!(cell(b, 3), "사업관리팀장");
    assert_eq!(cell(b, 4), "저녁메뉴 장보기 3월 진도 보고");
    // the LLM's own markers are replaced by the core's; 끝. after the last 붙임 line
    assert_eq!(
        cell(b, 5),
        "1. 관련: 저녁메뉴 장보기 용역 계약서 제7조\n2. 위 호와 관련하여 3월 진도를 아래와 같이 보고합니다.\n  가. 공정: 계획 대비 정상\n  나. 인력: 4명 투입\n  다. 다음 달 계획: 현장 조사\n\n붙임  1. 월간 진도 보고서 1부.\n      2. 사진 대지 1부.  끝."
    );
    assert_eq!(cell(b, 6), "개좋소 대표이사"); // 명의 default
    assert_eq!(cell(b, 7), "시행  개좋소-2026-031 (2026.03.02.)\n전화 000-0000-0000\n이메일 kim@example.com\n담당 김철수");
    let f = finalize(&d, 0).unwrap();
    assert_eq!(f.receipt["structure"]["regressions_vs_template"], json!([]));
    assert!(Package::from_bytes(f.bytes).unwrap().drop_stale_linesegs().unwrap().is_none());
}

#[test]
fn letter_minimal_ends_after_body_and_warns_on_missing_doc_no() {
    let e = engine();
    let s = json!({
        "doc_date": u("2026-03-02"), "recipient": llm("김밥천국공사 사장"), "doc_title": llm("현장 조사 일정 협조 요청"),
        "body": llm("현장 조사 일정에 대한 협조를 요청합니다."),
    });
    let r = e.plan_check(&plan(s.clone()));
    assert!(r.ok && r.finalizable, "{:?} {:?}", r.errors, r.blockers);
    let doc_no = r.documents[0].rules.iter().find(|x| x.id == "doc-no-missing").unwrap();
    assert_eq!(doc_no.status, "fail");
    let d = e.draft(&plan(s)).unwrap();
    let b = &d.documents[0].bytes;
    assert_eq!(cell(b, 5), "1. 현장 조사 일정에 대한 협조를 요청합니다.  끝.");
    assert_eq!(cell(b, 7), "시행  (2026.03.02.)"); // no empty lines for absent contacts
    // one attachment: no number
    let mut s2 = full();
    s2["attachments"] = llm("월간 진도 보고서 1부.");
    s2.as_object_mut().unwrap().remove("items");
    let d = e.draft(&plan(s2)).unwrap();
    assert!(cell(&d.documents[0].bytes, 5).ends_with("보고합니다.\n\n붙임  월간 진도 보고서 1부.  끝."));
}

#[test]
fn letter_reports_missing_recipient_and_refuses_end_mark_in_prose() {
    let e = engine();
    let mut s = full();
    s.as_object_mut().unwrap().remove("recipient");
    let r = e.plan_check(&plan(s));
    assert_eq!(r.documents[0].missing_required, vec!["recipient".to_string()]);
    let mut s = full();
    s["body"] = llm("보고합니다.  끝.");
    let r = e.plan_check(&plan(s));
    assert!(!r.ok, "끝. is the core's mark");
}

#[test]
fn letter_date_missing_keeps_doc_no_and_dates_in_body_survive() {
    // reviewer #4: a missing date must not drop the given document number; it is a required fact
    let e = engine();
    let s = json!({
        "doc_no": u("A-1"), "recipient": llm("김밥천국공사 사장"), "doc_title": llm("착수보고회 결과 보고"),
        "body": llm("2026. 3. 2. 착수보고회 결과를 보고합니다.\n-5% 감소 추세를 보고합니다."),
    });
    let r = e.plan_check(&plan(s.clone()));
    assert_eq!(r.documents[0].missing_required, vec!["doc_date".to_string()]);
    let d = e.draft(&plan(s)).unwrap();
    let b = &d.documents[0].bytes;
    assert_eq!(cell(b, 7), "시행  A-1 ");
    // reviewer #3: a leading date and a sign are content (D4-2)
    assert_eq!(cell(b, 5), "1. 2026. 3. 2. 착수보고회 결과를 보고합니다.\n2. -5% 감소 추세를 보고합니다.  끝.");
}

/// `OH_WRITE_EXAMPLES=<dir>` writes the library example from the anonymized inputs above.
#[test]
fn write_letter_example_when_asked() {
    let Ok(dir) = std::env::var("OH_WRITE_EXAMPLES") else { return };
    let d = engine().draft(&plan(full())).unwrap();
    std::fs::write(std::path::Path::new(&dir).join("client-report-letter-example.hwpx"), finalize(&d, 0).unwrap().bytes).unwrap();
}
