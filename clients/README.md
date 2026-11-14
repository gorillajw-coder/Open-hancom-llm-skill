# 클라이언트별 연결

모든 클라이언트가 같은 `oh.exe` 코어와 로컬 STDIO MCP 도구를 사용합니다. 설치기·안내만 구분합니다.

| 클라이언트 | 설치 안내 | Windows 설치기 |
|---|---|---|
| Claude Desktop | [Claude](claude/README.md) | `clients\claude\install.cmd` |
| ChatGPT Desktop | [ChatGPT](chatgpt/README.md) · [Español](chatgpt/GUIA_ES.md) | `clients\chatgpt\install.cmd` |
| Codex 앱·CLI·IDE | [Codex](codex/README.md) | `clients\codex\install.cmd` |

설치기는 **전체 압축을 푼 배포 폴더**에서 실행합니다. `oh.exe`, `library/`, `clients/`를 함께 둡니다. 소스 저장소에는 실행 파일이 없으므로 릴리스 ZIP을 받거나 직접 빌드해야 합니다.

기본 설치 위치는 `%LOCALAPPDATA%\OpenHancom`, 입력은 문서 폴더의 `OpenHancom\inbox`, 결과는 `OpenHancom\outbox`입니다. 기존 개인 카드·연결 팩·문서는 보존합니다. 같은 서버 항목을 다시 등록하면 해당 클라이언트가 참조하는 경로가 갱신됩니다. 이전에 사용자 지정 경로를 썼다면 그 경로를 유지해 설치하세요.

ChatGPT Desktop과 Codex는 같은 호스트의 MCP 설정을 공유하므로 둘 중 하나에서 등록하면 됩니다. 공개 묶음에는 공문 예시 카드 하나만 포함합니다. 회사 양식·사규·개인 문서는 공개하지 않습니다.

## 배포 묶음 만들기

배포 담당자는 같은 버전의 **공개 소스 체크아웃**에서 기존 **공개용** Windows ZIP을 입력으로 실행합니다. Python 3.11 이상은 묶음을 만드는 PC에만 필요합니다.

```sh
python scripts/make-client-kits.py open-hancom-1.1.0-windows-x64-public.zip dist/clients
```

같은 실행 파일·공개 라이브러리로 `…-claude.zip`, `…-chatgpt.zip`, `…-codex.zip`과 SHA256 파일을 만듭니다. 공개 소스의 라이브러리와 모든 바이트를 대조해 내부용·변조된 라이브러리가 든 ZIP은 거부합니다. 기존 릴리스 자산을 자동으로 바꾸거나 업로드하지 않습니다.
