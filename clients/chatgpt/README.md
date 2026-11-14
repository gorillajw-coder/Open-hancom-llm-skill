# ChatGPT Desktop

각자의 PC에서 `oh.exe`를 로컬 STDIO MCP로 실행합니다. 서버 URL이나 원격 호스팅은 필요하지 않습니다. 이 안내는 로컬 MCP를 지원하는 데스크탑 환경용이며 웹 채팅에 실행 파일 경로를 등록하는 방식이 아닙니다.

1. Windows 공개 ZIP 전체를 풀고 ChatGPT Desktop·Codex를 종료합니다.
2. `clients\chatgpt\install.cmd`를 실행합니다. 기존 설정을 백업하고 `open-hancom` 항목만 등록합니다. `CODEX_HOME`이 있으면 그 위치를 사용합니다.
3. 앱을 다시 열고 MCP 서버를 활성화합니다. Terra가 제공되는 계정이면 선택합니다.
4. [사용 프롬프트](PROMPT_ES.txt)를 붙여 넣고 `oh_guide`가 실제 호출되는지 확인합니다.

자동 등록이 보이지 않으면 Settings → MCP servers → Add server → STDIO에서 설치 결과의 Binary 경로와 아래 인수를 사용합니다. 경로에 공백이 있으면 따옴표로 묶습니다.

```text
mcp --library "<설치 폴더>\library" --workdir "<작업 폴더>"
```

`CODEX_HOME`의 `config.toml`(기본 `~/.codex/config.toml`)은 같은 호스트의 Codex와 공유합니다. 메뉴가 없으면 앱·계정·조직 정책의 로컬 MCP 지원을 확인하세요. 설치기의 verified는 설정 저장 확인이며 실제 도구 호출 성공과 구분합니다.

도구 실행 확인이 뜨면 요청한 조회·작성·저장 범위에서 허용합니다. 관리자 권한·전체 디스크 허용·보안 전역 해제는 필요하지 않습니다. 클라우드 첨부는 PC의 `inbox`로 자동 복사되지 않습니다.

[스페인어 간단 설명서](GUIA_ES.md). 설명서 언어와 관계없이 생성 문서는 기존 **한국어 양식**을 사용합니다.

공식 연결 안내: https://learn.chatgpt.com/docs/extend/mcp (2026-11-14 확인)
