# 라이브러리 만들기 설명서

이 문서는 Open-hancom에 새 문서 유형을 넣는 방법을 설명한다. 대상은 두 가지다.
- **개인 라이브러리**: 내 컴퓨터에서만 쓰는 템플릿과 카드 (D12)
- **공용 라이브러리 팩**: 팀이나 회사가 같이 쓰려고 묶어서 나눠 주는 라이브러리. 받는 사람이 소켓에 꽂아 쓴다 (D12)

> 상태 표시: **[현재]**는 지금 코어에서 동작하는 것이다. **[예정]**은 설계는 정했지만 아직 구현하지 않은 것이다. 카드 필드는 코어가 확장되면서 늘어난다. 정확한 목록은 맨 아래 '필드 참고'와 코어 버전을 같이 본다.

## 1. 라이브러리가 뭔가
라이브러리는 코어가 문서를 만들 때 쓰는 자료 묶음이다. 프로그램 코드가 아니라 **데이터**다. 개인화는 대부분 라이브러리만 바꿔서 끝난다. 코어 코드를 고쳐야 하는 경우는 6절에 있다.

```
<라이브러리 폴더>/
  manifest.toml        파일 목록과 sha256, 버전, 익명화 여부
  defaults.toml        입력이 없을 때 쓰는 표시된 기본값 (선택)
  cards/<id>.toml      문서 유형 카드. 슬롯, 계산, 위치, 규칙
  templates/<id>.hwpx  빈 양식. 카드가 sha256으로 고정한다
  regulations/*.md     규정 근거. 룰의 cite가 가리킨다
  regulations/*.toml   단가표 같은 조회용 규정 데이터 (코어 버전에 따라 다름)
  examples/*.hwpx      완성 예시. 시험과 LLM 참고용이고 익명화 필수
```

## 2. 소켓: 라이브러리를 어디에 꽂나
| 층 | 위치 | 누가 만드나 | 우선순위 |
|---|---|---|---|
| 개인 | `<설치 폴더>/personal-library/` | 본인(등록 흐름, 지금은 손으로) | 1 (가장 높음) |
| 공용 팩 | 사용자가 연결한 폴더나 zip (`<설치 폴더>/sockets.toml`에 기록) | 팀·회사 | 2 (먼저 연결한 팩이 앞, 새로 연결한 팩은 팩 중 가장 낮다) |
| 기본 | 설치할 때 같이 오는 `<설치 폴더>/library/` | 배포자 | 3 |

**[현재, 코어 0.4.0]** 코어는 세 층을 동시에 읽는다. 설치 폴더는 `library/`가 들어 있는 폴더다(`oh mcp --library <설치 폴더>/library`). 개인 폴더와 `sockets.toml`이 없으면 예전처럼 기본 라이브러리 하나만 쓴다.

- **연결·해제** (설치 폴더의 `oh`로 실행. 다른 위치면 `--root <설치 폴더>`):
  ```
  oh library add --path "D:\팀 라이브러리\공용 팩.zip" [--name 팀-공용]
  oh library list
  oh library remove --name 팀-공용
  ```
  이름을 안 주면 manifest의 `name`, 없으면 파일·폴더 이름(공백은 `-`)을 쓴다. 이름은 1~40자 글자·숫자·`-_.`이고 `base`·`personal`은 못 쓴다. 해제는 연결만 지우고 팩 파일은 지우지 않는다. 연결·해제 뒤에는 클라이언트(Claude Desktop 등)를 다시 시작해야 MCP 서버가 새 층을 읽는다.
- **검사**: 팩은 `manifest.toml`에 적힌 모든 파일의 sha256이 맞아야 하고 `anonymized = true`여야 한다(개인 라이브러리는 익명화하지 않아도 된다). 연결할 때 manifest 자체의 sha256을 `sockets.toml`에 고정한다. 나중에 팩 내용이 바뀌면(새 판 포함) `pack_changed`로 그 팩만 빠지고, 내용을 확인한 뒤 `oh library add`를 같은 이름으로 다시 하면 새 판이 고정된다. 연결 때 팩 카드가 실제로 묶이는지(템플릿 해시·앵커·틀 상속)도 확인하고, 실패하면 기록하지 않는다. 새 팩 때문에 전에 로드되던 다른 층(예: 그 팩이 바꾼 틀을 쓰는 개인 카드)이 빠지게 되면 역시 `pack_rejected`로 거부한다. 해제 뒤 빠지는 층이 생기면 `warnings`로 알린다.
- **충돌**: 같은 카드 id·틀 id·규정표 id·기본값 키가 여러 층에 있으면 높은 층 것을 쓰고 `conflicts`로 알린다(`{"kind": "card", "id": ..., "used": "personal", "shadowed": ["팀-공용", "base"]}`). 팩 카드는 기본 라이브러리의 틀(`approval-memo`, `official-letter`)을 `extends`로 쓸 수 있다.
- **고장 격리**: 연결한 팩이나 개인 라이브러리가 없어졌거나 해시가 틀리면 그 층만 빠지고 이유가 `status`에 남는다. 기본 라이브러리가 깨지면 전체가 멈춘다.
- **보이는 곳**: `oh_guide`의 `cards[].layer`·`library_layers`(층 목록·상태·충돌), `oh_library_search`의 `hits[].layer`·`conflicts`, `oh_card_get`의 `layer`, `oh doctor`의 `library_layers` 점검.
- **zip 팩 구조**: zip 맨 위에 `manifest.toml`이 있거나, 맨 위 폴더 하나 안에 있어야 한다(`team-pack/manifest.toml`). zip은 256MB, 항목 2만 개, manifest에 적힌 파일 하나는 16MB, 풀린 합계 256MB까지. manifest에 없는 항목은 풀지 않는다. 경로 구분자는 `/`여야 한다(역슬래시 경로 zip은 `invalid_pack`).
- **틀을 덮는 팩**: 팩이 기본 틀(`official-letter` 등)을 같은 id로 덮으면 그 틀을 쓰는 기본 카드도 바뀐다. `conflicts`에는 `frame`으로만 나온다.
- **팩 manifest 예**:
  ```toml
  version = "1.0.0"
  name = "팀-공용"        # 선택: 연결 이름 기본값
  anonymized = true       # 공용 팩은 필수
  [[file]]
  path = "cards/team-reply-letter.toml"
  sha256 = "<sha256>"
  kind = "card"           # card | frame | template | example | regulation | regulation_table | defaults
  ```
  `defaults.toml`은 팩에 없어도 된다. 있으면 같은 키는 아래 층 값을 덮고(충돌 보고), 새 키는 추가된다.
- **규정 문서(.md)**: 카드·틀·규정표(.toml)와 달리 `regulations/*.md`는 id로 가려지지 않는다. 층마다 검색 결과에 같이 나온다. 카드의 `sources`가 가리키는 것은 그 카드가 속한 층의 파일이다.

## 3. 새 문서 유형 만들기 (손으로 할 때)
등록 흐름(5절)이 생기면 2~5단계 대부분을 코어와 LLM이 대신한다. 지금은 손으로 한다.

1. **양식 준비.** HWPX로 저장한다. HWP만 있으면 한글에서 "다른 이름으로 저장 → HWPX"로 바꾼다. 값이 들어간 완성본 1부와 빈 양식 1부가 있으면 가장 좋다.
2. **구조 확인.** `oh inspect <양식.hwpx>`로 문단·표·셀 구조를 본다. 바꿀 자리마다 근처의 **고정 라벨 글자**를 찾는다(예: "작성일 : ", "제목", "(1) 계약명: "). 같은 양식이라도 표 열 수와 본문 방식이 판본마다 다를 수 있다. 그래서 위치는 좌표보다 라벨로 잡는다.
3. **슬롯 정하기.** 문서에 들어갈 사실 하나하나가 슬롯이다.
   - 첨부나 사용자 입력에서 오는 사실에는 `require_source = true`를 단다.
   - 사유·목적 같은 문장은 `kind = "prose"`로 하고 `guide`, `max_chars`, `forbidden`을 단다.
   - 기본값이 필요한 슬롯만 `default_ref`로 `defaults.toml`을 가리킨다. 발주기관이나 부서명처럼 지어내면 안 되는 사실에는 기본값을 두지 않는다 (D11).
4. **계산 정하기.** 합계·환산·비율·일수처럼 계산으로 나오는 값은 LLM이 넣지 않는다. 카드의 `[[calc]]`로 코어가 계산한다 (D4). 원문 값이 따로 있으면 `compare_to`로 대조만 한다.
5. **위치(target) 쓰기.** 슬롯을 문서의 어느 글자에 어떤 형식으로 넣을지 적는다. 형식은 `fmt`에 `{슬롯|필터}`로 쓴다. 대괄호 `[...]`로 감싼 부분은 그 안의 슬롯이 하나라도 비어 있으면 통째로 빠진다(한 겹만 가능).
6. **규칙 쓰기.** `[[rule]]`에 조건(`when`), 확인(`assert` 또는 `require`), 심각도(`block`/`warn`), 메시지, 근거(`cite`)를 적는다. `block`이면 최종 저장을 막는다.
7. **manifest 갱신.** 새 파일을 `[[file]]`로 넣고 sha256을 적는다. 카드의 `template_sha256`도 양식 해시와 맞춘다.
8. **시험.** 아래 7절 체크리스트를 통과시킨다.

### 최소 카드 예
```toml
id = "my-memo"
title = "내 품의서"
version = "1.0.0"
template = "templates/my-memo.hwpx"
template_sha256 = "<양식 sha256>"
triggers = ["내 품의", "OO 품의서"]

[[slot]]
id = "doc_date"
label = "작성일"
kind = "date"
require_source = true

[[slot]]
id = "title_text"
label = "제목"
kind = "prose"
required = true
max_chars = 60

[[target]]
id = "doc_date"
fmt = "{doc_date|date_kr}"
anchor = [{ starts = "작성일 : " }]

[[target]]
id = "title"
fmt = "{title_text}"
anchor = [{ after = "제목", n = 1 }]
```
실제 예는 `library/cards/client-report-letter.toml`이 짧고 주석도 달려 있다.

## 4. 익명화 (공용 팩·배포용이면 필수)
개인 라이브러리는 실제 값을 그대로 둬도 된다. 남에게 주는 팩은 D6 규칙대로 바꾼다.

| 항목 | 치환값 |
|---|---|
| 인명 | 김철수 |
| 계좌번호 | XXXXXXXXXX |
| 전화번호 | 000-0000-0000 |
| 이메일 | kim@example.com |
| 주민번호·여권번호 | 000000-00000 |
| 주소 | 태양계 내행성시 금성동 |
| 우리 회사 | 개좋소 |
| 상대 회사 | 김밥천국 |
| 사업명 | 저녁메뉴 장보기 |

- 금액은 사업 단위로 같은 배율을 곱한다. 계산값은 코어가 다시 계산한다.
- 날짜는 7의 배수 일수만큼 옮긴다. 그래야 요일이 유지된다.
- HWPX 안의 `Preview/PrvText.txt`, `Preview/PrvImage.png`, 작성자 메타, 내장 이미지에도 원문이 남는다. 이것들도 정리한 뒤 잔존 스캔을 한다. 스캔은 고유명, 금액 원문, 계좌 패턴을 모든 ZIP 멤버에서 찾는다.
- manifest에 `anonymized = true`는 스캔 결과가 0일 때만 적는다.

## 5. 등록 흐름 **[예정]** (D10)
사용자가 양식을 주면 다음 순서로 카드가 만들어진다.
1. 코어가 구조를 뽑는다. 칸·표 좌표, 라벨-값 쌍, 숫자·날짜 패턴, 합계 행 후보가 대상이다.
2. LLM이 카드 초안을 쓴다. 슬롯 후보, 서술 지침, 계산식 후보, 룰 후보다.
3. 코어가 앵커가 하나씩만 걸리는지 확인한다. 계산식을 원본 숫자에 대입해 맞는지 본다. 왕복 시험(원본 값으로 다시 만들면 원본과 같은지)과 익명화를 한다.
4. 사용자가 슬롯 목록, 모호한 해석, 룰 심각도, 저장 위치를 확인한다.
5. `commit`하면 개인 라이브러리에 저장된다. 덮어쓰기는 이 절차로만 한다 (D4-7).

## 6. 코어 코드를 고쳐야 하는 경우
데이터만으로 안 되는 경우는 다음과 같다.
- 새 계산 함수가 필요할 때 (누진 구간, 특수 반올림, 달력 규칙)
- 새 문서 틀이 필요할 때
- 열 수가 바뀌는 표처럼 코어의 표 모델 밖일 때
- 외부 데이터가 필요할 때 (환율 API 등은 외부 효과라 사용자 결정 필요)

지키는 원칙:
- 새 함수는 코어의 허용 목록에 순수 함수로 넣는다. 따로 계산한 기대값으로 시험을 만든다. 경계값(0, 음수, 반올림 경계, 윤년)을 넣고 기존 시험을 전부 다시 돌린다.
- 실행 중에 LLM이 만든 코드를 불러 실행하지 않는다.
- 카드에 코어가 모르는 함수가 있으면 코어가 `unknown_function`으로 멈춘다. 이걸 코어 확장 요청으로 넘긴다.

## 7. 체크리스트
- [ ] `oh_card_get`으로 카드가 읽힌다.
- [ ] 예시 값으로 `oh_plan_check` → `oh_draft` → `oh_finalize`가 끝까지 간다.
- [ ] 영수증의 `regressions_vs_template`이 비어 있다. 앵커가 모두 한 번씩 걸렸다.
- [ ] 계산 슬롯에 LLM 값이 들어가지 않는다. 원문과 다르면 경고가 난다.
- [ ] `block` 룰을 일부러 어긴 입력에서 finalize가 거절된다.
- [ ] 한컴(가능하면)에서 결과 파일이 열리고 쪽수가 맞다.
- [ ] 공용 팩이면 익명화 잔존 스캔 0이고 manifest 해시가 맞다. 빈 설치 폴더에 `oh library add --root <폴더> --path <팩>`이 `loaded`로 통과하고, `conflicts`에 의도하지 않은 id 겹침이 없다.

## 필드 참고 (코어 버전에 따라 늘어난다)
- **슬롯 kind** [현재]: text, person, prose, money, decimal, date, email, phone, list(객체 배열), file(inbox 파일 이름, 코어가 바이트를 읽음)
- **슬롯 옵션** [현재]: required, require_source, default, default_ref, default_expr(코어가 계산한 기본값, 기본값으로 표시), default_style, guide, max_chars, forbidden, multiline(prose, 줄마다 문단), max_items(list)
  - list 필드(`[[slot.field]]`): id, label, kind, required, max_chars, one_of. `llm` 출처는 모든 필드가 prose인 목록만.
- **문서 위치** [현재]
  - `[[target]]` 글자 덩어리 앵커: `starts`(유일), `after` + `n`, `follow`, `keep_prefix`, 여러 앵커는 `wrap_width`·`continuation_indent`
  - `[[cell]]` 표 셀 통째: `table`(문서 순서 표 번호, 중첩 포함), `row`, `col`, `fmt`(`\n` = 새 문단), `keep_if_unresolved`. 빈 셀도 채운다. 그림·컨트롤·중첩 표가 있는 셀은 거부.
  - `[[cell]]` 라벨 앵커(코어 0.3.0): `table`·`row`·`col` 대신 `label`(그 칸의 글자, 공백 무시, 문서 안에서 유일해야 함) + `dir`(`right` 기본 / `below`). 값은 라벨 칸의 오른쪽(또는 아래) 칸에 쓴다. 머리 표 열 수가 다른 같은 양식에서도 맞는다. 여러 곳이면 `anchor_ambiguous` → `table`로 좁힌다. 중첩 표가 든 칸·라벨 칸 자체는 쓰지 않는다.
  - `[[rows]]` 반복 행: `table`, `list`(list 슬롯), `first`, `template_rows`, `cols`(열마다 fmt, `{item.필드}`). 템플릿 행을 복제·삭제한다.
- **형식 문법**: `{id}`·`{id|필터}`, `[ ... ]`는 안의 값이 하나라도 없으면 통째로 뺀다. 글자 그대로 `[`·`]`·`{`·`}`·`\`가 필요하면 앞에 `\`를 붙인다(TOML에서는 작은따옴표 문자열 `' \[사업코드 {project_code}\]'`이 편하다).
- **형식 필터** [현재]:
  - 금액: usd, krw, usd2, pct
  - 날짜: date_kr, date_kr0, date_dot, date_dot_wd(2026.11.20(금)), md_wd(11.20(금))
  - 줄: nobullet, dot(∙ ), dash(- )
  - 공문 번호(0.4.0): num(줄마다 1. 2. …), ganada(줄마다 두 칸 들여 가. 나. … 하. 거. …, 28개 넘으면 숫자), attach(붙임: 한 줄이면 번호 없이, 여러 줄이면 1. 2. 로 매기고 둘째 줄부터 6칸 들임). LLM이 줄 앞에 쓴 번호·기호(1~2자리 숫자+`.`/`)`, 가. 나), ①~⑳, `- ` `∙ ` `• ` `· ` `* `)는 지우고 코어 번호로 바꾼다. 번호·기호 뒤에 공백이 있어야 지운다. 그래서 "1.5배", "2026. 3. 2.", "-5%"는 그대로다. 숫자 번호를 떼고 남은 앞부분이 다시 `1~2자리.`(뒤에 공백·`(`·끝)이면 월·일 날짜("3. 15.(금)")로 보고 번호를 떼지 않는다(LLM이 "1. 3. 15."처럼 번호를 직접 쓰면 번호가 두 번 보인다). 전각 숫자(１.)는 지우지 않는다.
- **계산 함수** [현재] (값은 문자열: 숫자는 십진, 날짜 YYYY-MM-DD, 목록은 JSON)
  - 수: sum, mul, sub, div, neg, abs, fx(원 반올림), vat, round_krw, pos, if_pos, if_eq, coalesce(빈 값 건너뜀)
  - 날짜: date_add, days, weekday
  - 규정표(`regulation = "<id>"`): grade, grade_basis, rate(kind, grade, title), discount(nights), reg_status, discount_status
  - 목록: segs, nights, per_diem, per_diem_note, grades, grades_ok, segs_check, seg_join, items_krw, items_usd, items_note, itin_check, list_len
  - 반올림·비율(0.3.0): `round_to(x, unit, mode)` — unit의 배수로, mode `'half_up'`(0.5는 0에서 먼 쪽) / `'floor'` / `'ceil'`(음수는 수직선 방향). 예: 백만원 소수 둘째 자리 `round_to(div(x, 1000000), 0.01, 'half_up')`. `pct(x, p)` = x × p/100(반올림 없음)
  - 분담(0.3.0): `share(total, p, unit, mode)` 한쪽 몫, `share_rest(total, p, unit, mode)` = total − share(나머지 규약: 두 몫의 합이 항상 합계와 같다)
  - 기간 요율(0.3.0): `prorate(amount, 연요율%, 개월)` = amount × 요율/100 × 개월/12(반올림 없음, `round_to`로 감싼다)
  - 개월(0.3.0): `add_months(date, n)`(넘치는 날은 그달 말일: 01-31 + 1 = 02-28/29), `months_between(a, b)`(add_months(a, m) ≤ b인 가장 큰 m, b < a면 계산 안 됨). 계약기간 개월 수는 `months_between(start, date_add(end, 1))`
  - 목록 합(0.3.0): `list_sum(list, 'field')`(모든 항목에 그 필드가 있어야 함)
- **룰 함수** [현재]: present, absent, any_present, date_le, `id == '값'` / `!=`, `and`, 수 비교(0.3.0) `le(a, b)`(a ≤ b), `near(a, b, tol)`(|a − b| ≤ tol). 인자는 슬롯·계산 id나 숫자. 값이 없으면 거짓. 합계=행합 허용오차는 계산 `compare_to`(다르면 경고) + `near(..., 1)` block 룰(허용오차 넘으면 막기)로 짠다.
- **카드 연결** [현재]: next(안내), follows(같은 계획에 앞 문서 필요), `[carry] from_card` + `[carry.map]`(같은 계획 앞 문서의 슬롯·계산값을 가져옴. 앞 문서 기본값은 기본값으로 표시, 명시값이 다르면 carry_mismatch 경고)
- **xlsx** [현재]
  - 읽기 `[[extract]]`: from(file 슬롯), sheet, section, label_col, total_col, total_marker, value_col. 항목 합을 쓰고 시트 소계와 다르면 경고.
  - 쓰기 `[xlsx]` template·template_sha256·sheet + `[[xlsx.cell]]` at, fmt, number. 템플릿에 있는 칸에만 쓴다. 수식은 보존하고 캐시값을 코어 값으로, 열 때 재계산.
- **규정표** [현재]: 라이브러리 `regulations/<id>.toml`(manifest kind `regulation_table`, status sample). 작업 폴더 `regulations/<id>.toml`(status company)이 있으면 대체, 잘못되면 무시하고 노트.
- **공통 틀** [현재, 0.3.0]: `frames/<id>.toml`(manifest kind `frame`)에 slot·calc·target·cell·rule·`[project]`를 둔다. 템플릿은 없다. 카드 `extends = "<id>"`로 상속하면 틀 항목이 앞에, 카드 항목이 뒤에 붙고, 같은 id는 카드 것이 이긴다. `omit = ["id", ...]`로 틀 항목을 뺀다(틀에 없는 id면 거부). 기본 틀 `approval-memo`: 작성일·작성 부서·작성자(직위)·문서번호·사업명(+사업코드)·시행일·제목·우리 회사, 시행일<작성일 경고. 유형 카드는 본문·표·계산·룰만 쓴다.
- **공문 틀** [현재, 0.4.0]: `frames/official-letter.toml`. 결재표 없음. 슬롯: 시행 날짜(필수)·문서번호(없으면 경고)·수신·경유·참조·제목·본문(`body`, 줄마다 1.)·세부(`items`, 줄마다 가.)·붙임(`attachments`)·발신 기관(우리 회사)·명의 직위(기본 "대표이사", 표시된 기본값)·주소·전화·이메일·담당자. 본문 칸 fmt는 `{body|num}[\n{items|ganada}][\n\n붙임  {attachments|attach}]  끝.`이라 "끝."이 마지막 줄 뒤 두 칸에 붙는다. 머리(수신·(경유)·참조·제목)는 라벨 셀, 기관명·본문·명의·시행 칸은 기본 템플릿 좌표(표 0의 0·5·6·7행)다. 다른 공문 양식이면 같은 id의 `[[cell]]`을 카드에서 다시 선언한다. 예시 카드 `client-report-letter`(발주기관 앞 보고·요청). 템플릿은 `scripts/official-letter-spec.json`을 `oh create`로 만든 익명 양식이다.
- **필요 코어** [현재, 0.3.0]: `requires_core = "x.y.z"`. 코어가 낮으면 `core_too_old`로 카드를 거부한다.
- **사업 기록** [현재, 0.3.0, D11]: 카드(또는 틀) `[project] map = { 슬롯 = "사실키" }`. plan 최상위 `"project": "사업 키"`가 있으면 호스트가 작업 폴더 `projects/<키>.json`을 읽고, 코어가 빈 슬롯을 `memory:<키>.<사실> ← 원출처`로 채운다(명시값 > carry > memory > 기본값). 명시값이 기록과 다르면 `project_fact_mismatch` 경고. 확정 때 provided 값만 기록에 저장한다(기본값·memory 값은 저장 안 함). 발주기관·부서에는 기본값을 두지 않는다.
