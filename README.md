# Open-hancom-llm-skill

LLM 클라이언트(Claude Desktop·Claude Code·Codex 등)에게 **한글(HWPX) 결재서류를 만들게 하는 로컬 도구**입니다.
사용자가 "발주기관에 3월 진도 보고 공문 만들어줘"처럼 자연어로 요청하면, LLM은 계획과 검토만 하고 문서 작성·계산·검증은 Rust 코어가 합니다.

- **LLM이 하지 않는 일**: HWPX/XML 직접 편집, 합계·환산 같은 계산, 표 구조·서식 변경, 검증 통과 선언, 출처 없는 사실 지어내기. 이런 일은 코어와 카드 규칙이 맡습니다.
- **코어가 하는 일**: 문서 카드(슬롯·계산·룰·위치)에 따라 템플릿을 채우고, 결과와 영수증(무엇이 어디에 들어갔는지, 어떤 룰이 통과·실패했는지)을 남깁니다.
- **연결 방식**: stdio MCP 서버(`oh mcp`)와 CLI(`oh`). 대상 OS는 Windows이고, Linux에서도 빌드·시험됩니다.

> 상태: **1.1.0** (2026-11-13). 이 공개 저장소의 기본 라이브러리에는 **공문 틀(`official-letter`)과 예시 카드 하나**만 들어 있습니다. 회사 양식에서 만든 카드는 공개하지 않습니다. 자기 양식은 등록 흐름(`oh_library_register` → 확인 → `oh_library_commit`)으로 개인 라이브러리에 넣거나 공용 팩으로 연결합니다(`docs/LIBRARY-GUIDE.md`). 변경 기록은 `CHANGELOG.md`.

## 받기 (Windows)
[Releases](../../releases)에서 `open-hancom-<버전>-windows-x64-public.zip`을 받습니다. 실행 파일은 코드 서명이 없습니다(첫 실행 때 Windows 경고가 뜰 수 있음, `INSTALL.md` 참조). zip의 `VERSION.txt`에 `oh.exe` sha256이 있습니다.
- **Claude에게 맡기기**: Claude Desktop의 Code 탭(또는 Claude Code)에 zip을 주고 "zip 안의 CLAUDE-SETUP.md 대로 설치해 줘"라고 합니다.
- **직접**: zip 차단 해제 → 압축 풀기 → `install-claude.cmd` 실행 → Claude를 트레이까지 종료했다가 다시 켭니다.

### ChatGPT · Codex · Claude

코어는 하나이며 클라이언트별 설치·안내는 [`clients/`](clients/README.md)에서 관리합니다.

| 사용 환경 | 안내 |
|---|---|
| Claude Desktop·Claude Code | [Claude 설치](clients/claude/README.md) |
| ChatGPT Desktop | [ChatGPT 설치](clients/chatgpt/README.md) · [설명서 Español](clients/chatgpt/GUIA_ES.md) |
| Codex 앱·CLI·IDE | [Codex 설치](clients/codex/README.md) |

현재 v1.1.0의 기존 공개 ZIP에는 새 `clients/` 폴더가 없습니다. 해당 ZIP을 푼 폴더에 이 저장소의 `clients/`를 함께 두거나, 배포 담당자가 [`make-client-kits.py`](scripts/make-client-kits.py)로 클라이언트별 ZIP을 만들면 됩니다. 기존 릴리스·코어 버전은 그대로입니다. 공개용 기본 라이브러리는 공문 예시 하나이며, 문서는 한국어로 작성합니다.

## 빌드
```
cargo build --release
cargo test --locked
```
Windows 배포용은 `RUSTFLAGS=-C target-feature=+crt-static`을 권장합니다(런타임 DLL 의존 제거).

## 설치와 연결
```
oh install --client claude-desktop     # 또는 claude-code, codex, chatgpt-desktop --config <경로>
oh doctor
```
- Claude Desktop은 **트레이까지 완전히 종료한 뒤** 설치합니다. 실행 중이면 앱이 종료하면서 설정을 다시 써서 등록이 사라질 수 있어서 `client_running`으로 거절합니다.
- `PROMPT.md`가 LLM 진입 지침입니다. `oh_guide` 도구도 같은 내용을 줍니다.
- 자세한 내용은 `INSTALL.md`를 보세요.

## 라이브러리
- 라이브러리는 데이터입니다: 카드(`cards/*.toml`), 공통 틀(`frames/`), 템플릿(`templates/*.hwpx`), 규정(`regulations/`), 예시. `manifest.toml`에 모든 파일의 sha256을 적고, 로딩할 때 검사합니다.
- 층: **개인 라이브러리 > 연결한 공용 팩(폴더·zip) > 기본 라이브러리**. `oh library add/list/remove`로 연결합니다.
- 카드 작성법은 `docs/LIBRARY-GUIDE.md`에 있습니다.
- 팩 만들기(1.1.0): `oh library anonymize`(치환표로 HWPX 익명화), `oh library scan`(검사어·연락처·미리보기 잔존 검사), `oh library manifest`(해시 manifest 작성), `oh library check`(연결 전 검사). Python이 필요 없습니다. 치환표와 검사어 파일은 팩 밖에 둡니다.

## 구성
| 크레이트 | 역할 |
|---|---|
| `oh-document` | HWPX 패키지 모델·편집·생성 |
| `oh-recipe` | 카드 엔진: 슬롯·계산·룰·문서 위치, 영수증 |
| `oh-library` | 라이브러리 로딩·해시 검사·층 소켓·zip 팩 |
| `oh-host` | 파일시스템 계층 |
| `oh-mcp` | stdio MCP 서버 |
| `oh-cli` | `oh` 바이너리 |

## 라이선스
Apache License 2.0(`LICENSE`). 고지는 `NOTICE`, 의존 크레이트 목록은 `THIRD-PARTY-NOTICES`에 있습니다. `vendor/zune-jpeg`는 원 라이선스를 그대로 따릅니다.
"한글"과 HWPX는 한글과컴퓨터의 제품·형식입니다. 이 프로젝트는 한글과컴퓨터와 관련이 없습니다.
