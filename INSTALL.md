# Open-hancom 설치

배포물: `oh`(Windows는 `oh.exe`), `library/`, `PROMPT.md`, `INSTALL.md`. 관리자 권한은 필요 없다. 우회 플래그(`-ExecutionPolicy Bypass` 등)는 쓰지 않는다.

## 설치
1. 배포 zip을 풀고, 사용할 LLM 클라이언트를 **트레이까지 완전히 종료**한다. 실행 중인 Claude Desktop은 종료하면서 설정 파일을 다시 써서 방금 넣은 등록을 지울 수 있다(회사컴 실측: 등록 2분 뒤 `mcpServers`가 사라짐). Windows에서 Claude Desktop(`claude.exe`)이 돌고 있으면 `oh install`·`oh uninstall`(claude-desktop)은 `client_running`으로 거절한다. 같은 이름의 Claude Code 실행 파일(`%USERPROFILE%\.local\bin\claude.exe`, `.claude` 아래)은 제외한다. 프로세스 목록(tasklist)을 못 읽으면 거절하지 않고 `warnings`에 `client_running_unknown`을 남긴다. `claude.exe`는 보이는데 경로를 확인하지 못하면(PowerShell 차단 등) 앱으로 보고 거절한다. `--force-running`으로 강제할 수 있지만 경고가 남고 등록이 덮어써질 수 있다.
2. 풀린 폴더에서 클라이언트별로 한 번 실행한다.

```
oh install --client claude-desktop
oh install --client codex
oh install --client claude-code
oh install --client chatgpt-desktop --config <설정 파일 경로>
```

`oh install`이 하는 일
- 바이너리·`library/`·`PROMPT.md`·`INSTALL.md`를 설치 폴더로 복사한다. Windows `%LOCALAPPDATA%\OpenHancom\`, Linux `~/.local/share/open-hancom/`. 다르게 하려면 `--root <폴더>`.
- 작업 폴더를 만든다: Windows `문서\OpenHancom\`, Linux `~/OpenHancom/` (`--workdir <폴더>`로 변경). 하위에 `inbox`(첨부 투입) `work`(중간본) `outbox`(최종본).
- 클라이언트 MCP 설정에 `open-hancom` 항목을 추가한다. 기존 파일은 `<파일>.oh-backup-<시각>`으로 백업하고, 다른 서버 항목은 그대로 둔다. 설정 파일이 손상(JSON/TOML 오류)이면 아무것도 바꾸지 않고 오류로 끝난다.
- 설정 파일을 쓴 뒤 다시 읽어 `open-hancom` 항목이 있는지 확인하고 결과를 `verified`(true/false)로 알린다. `uninstall`은 항목이 사라졌는지 확인한다.
3. 클라이언트를 다시 켠다. 점검: `oh doctor`(앱이 실행 중이면 `client_registration`에 "덮어써질 수 있다" 경고가 붙는다. 앱을 켠 뒤에도 `registered: true`인지 본다).
4. `PROMPT.md` 내용을 클라이언트의 지침 자리에 붙인다(`oh_guide` 도구도 같은 내용을 준다). Claude는 프로젝트 지침, Codex는 `AGENTS.md`, ChatGPT 데스크톱은 프로젝트 지침.

## 클라이언트별 설정 위치
| 클라이언트 | 파일 | 형식 |
|---|---|---|
| claude-desktop | Windows `%APPDATA%\Claude\claude_desktop_config.json`, macOS `~/Library/Application Support/Claude/…`, Linux `~/.config/Claude/…` | JSON `mcpServers` |
| claude-code | `~/.claude.json`(`CLAUDE_CONFIG_DIR`가 있으면 그 폴더의 `.claude.json`) 최상위 `mcpServers`(user 범위) | JSON |
| codex | `~/.codex/config.toml`(`CODEX_HOME` 우선) `[mcp_servers.open-hancom]` | TOML |
| chatgpt-desktop | **기본 위치를 확인하지 못해 자동 설정하지 않는다.** 공식 문서(learn.chatgpt.com, MCP)는 앱의 설정 > MCP servers > Add server > STDIO 등록과 Codex `config.toml` 공유를 안내한다. `--config <경로>`를 주면 `.toml`은 TOML, 그 외는 `mcpServers` JSON으로 기록한다. 앱이 읽는지는 직접 확인한다. | |

수동 설정이 필요하면 서버 명령은 `<설치 폴더>/oh`, 인수는 `mcp --library <설치 폴더>/library --workdir <작업 폴더>`.

## 공용 라이브러리 연결 (소켓)
팀·회사가 나눠 준 라이브러리 팩(폴더나 zip)을 연결한다. 우선순위는 개인(`<설치 폴더>\personal-library`) > 연결한 팩(먼저 연결한 팩이 앞) > 기본 `library`.
- **설치 폴더의 `oh`로 실행한다**(`%LOCALAPPDATA%\OpenHancom\oh.exe`). 압축을 푼 배포 폴더의 `oh`로 실행하면 그 폴더에 연결이 기록돼 설치된 MCP 서버가 못 본다. 다른 위치에서 실행하면 `--root <설치 폴더>`를 준다.
- 연결: `oh library add --path "<팩 폴더나 zip>" [--name <이름>]`
- 목록·상태·충돌: `oh library list`
- 해제: `oh library remove --name <이름>` (팩 파일은 지우지 않는다)
- 연결은 `<설치 폴더>\sockets.toml`에 기록된다. 연결·해제 뒤 클라이언트를 다시 켠다. 자세한 규칙은 `docs/LIBRARY-GUIDE.md` 2절.

## 제거
`oh uninstall --client <이름>`은 설정 항목만 지운다(백업 생성, 다른 항목 보존). 설치 파일까지 지우려면 `--remove-files`(개인 라이브러리 `personal-library`와 `sockets.toml`은 남긴다). 작업 폴더(`문서\OpenHancom`)는 지우지 않는다.

## 문제 해결
- `source_locked`: 첨부를 다른 프로그램이 잠그고 있다. 그 프로그램에서 파일을 닫는다.
- `target_exists`: 같은 이름의 출력이 있다. 다른 출력 이름을 쓴다(덮어쓰지 않는다).
- SmartScreen·Defender가 막으면 파일 속성의 차단 해제나 보안팀 허용 절차를 쓴다. 코어는 우회하지 않는다.
- `oh doctor`가 어느 단계에서 실패했는지 JSON으로 알려 준다.
