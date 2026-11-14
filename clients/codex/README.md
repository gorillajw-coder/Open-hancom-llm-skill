# Codex 앱·CLI·IDE

Windows 공개 릴리스 ZIP 전체를 풀고 Codex·ChatGPT Desktop을 종료한 뒤 `clients\codex\install.cmd`를 실행합니다. 기존 설정은 백업하고 다른 MCP 항목을 보존합니다. 사용자가 클라이언트를 다시 열어 `oh_guide`가 실제 호출되는지 확인합니다.

직접 등록은 배포 폴더의 PowerShell에서 다음과 같습니다.

```powershell
.\oh.exe install --client codex
```

`CODEX_HOME`(기본 `~/.codex`)의 `config.toml`에 등록하며, 같은 호스트의 ChatGPT Desktop·CLI·IDE와 공유합니다. 이미 같은 서버를 등록했다면 중복 등록할 필요가 없습니다. 수동 등록을 선호하면 `codex mcp add open-hancom -- <oh 실행 파일 전체 경로> mcp --library <library 전체 경로> --workdir <작업 폴더>`를 사용합니다. 이 수동 명령은 실행 파일을 설치 폴더로 복사하지 않으므로 원래 파일을 이동하지 마세요.

기본 입력/출력은 문서 폴더의 `OpenHancom\inbox`와 `OpenHancom\outbox`입니다. MCP와 요청한 파일 작업에 필요한 권한만 허용하고, 샌드박스·승인 정책을 전역 해제하지 않습니다. 원격 호스트에서 실행하는 Codex라면 실행 파일과 폴더도 그 호스트에 있어야 합니다.

첫 요청: “open-hancom의 oh_guide부터 읽고 공문 예시 카드를 확인해 줘. 문서는 한국어로 작성하고 빠진 사실은 한 번에 물어봐. 계산·생성·검증은 코어에 맡겨.”

공식 연결 안내: https://learn.chatgpt.com/docs/extend/mcp (2026-11-14 확인)
