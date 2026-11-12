# Claude에게: Open-hancom 설치 절차

이 파일을 받은 Claude는 아래 순서대로 사용자의 Windows PC에 Open-hancom을 설치하고 MCP 서버(`open-hancom`)를 Claude Desktop에 등록한다. 관리자 권한은 쓰지 않는다. `-ExecutionPolicy Bypass` 같은 우회 플래그, 보안 프로그램 끄기, `--force-running`은 쓰지 않는다.

## 0. 내가 명령을 실행할 수 있는지 먼저 본다
- **명령을 실행할 수 있다**(Claude Desktop의 Code 탭이나 Claude Code처럼 PowerShell을 쓸 수 있다) → 1번부터 직접 한다.
- **명령을 실행할 수 없다**(일반 채팅) → 사용자에게 아래를 안내하고 끝낸다.
  1. zip 파일을 오른쪽 클릭 → 속성 → 아래쪽 "차단 해제"를 체크하고 확인을 누른다.
  2. zip을 `문서` 폴더에 푼다.
  3. 풀린 폴더의 `install-claude.cmd`를 더블클릭한다. 검은 창이 "Claude 앱을 종료해 주세요"라고 하면 Claude를 트레이 아이콘까지 종료한다(작업 표시줄 오른쪽 아래 아이콘 → 종료).
  4. 창에 "등록했습니다"가 나오면 Claude를 다시 켠다.
  5. 새 대화에서 "open-hancom 도구 목록 보여줘"라고 해 본다.
  - Windows "PC 보호" 창이 뜨면 "추가 정보" → "실행". 회사 보안 프로그램이 막으면 우회하지 말고 IT 담당자에게 `oh.exe` 허용을 요청한다(sha256은 `VERSION.txt`).
- **Cowork(가상 환경)에서 실행 중**이면 사용자 PC의 설정을 바꿀 수 없다. 위 안내로 넘어간다.

## 1. 압축 풀기
사용자가 준 zip 경로를 확인한다(보통 `다운로드`). PowerShell:
```powershell
$ProgressPreference = 'SilentlyContinue'; $ErrorActionPreference = 'Stop'
$zip = "<zip 경로>"
# 매번 새 빈 폴더에 푼다(예전에 푼 키트를 잘못 쓰지 않게)
$dst = Join-Path ([Environment]::GetFolderPath('MyDocuments')) ('OpenHancom-setup-' + (Get-Date -Format yyyyMMddHHmmss))
New-Item -ItemType Directory -Path $dst | Out-Null
Unblock-File -LiteralPath $zip
Expand-Archive -LiteralPath $zip -DestinationPath $dst
Get-ChildItem -LiteralPath $dst -Recurse -File | Unblock-File
$kits = @(Get-ChildItem -LiteralPath $dst -Recurse -Filter install-claude.cmd)
if ($kits.Count -ne 1) { throw "install-claude.cmd가 $($kits.Count)개다. zip을 확인한다" }
$kit = $kits[0].DirectoryName
Get-Content -LiteralPath "$kit\VERSION.txt"
& "$kit\oh.exe" schema | Out-Null; "oh.exe exit=$LASTEXITCODE"
```
- 오류가 나면(zip 없음·손상, 압축 해제 실패) 거기서 멈추고 오류 문장을 사용자에게 전한다. 이전에 풀어 둔 폴더로 진행하지 않는다.
- `VERSION.txt`의 버전이 사용자가 준 zip 이름의 버전과 같은지 본다.
- `oh.exe`가 막히면(종료 코드가 0이 아니거나 보안 경고) 0번의 보안 안내를 사용자에게 전하고 멈춘다.

## 2. 등록 예약 (Claude Desktop이 꺼지는 순간 등록)
Claude Desktop은 켜져 있는 동안 설정 파일을 다시 써서 등록을 지운다. 그래서 등록 스크립트를 **이 Claude 세션과 분리된 프로세스로** 띄워 두고, 사용자가 앱을 끄면 그때 등록되게 한다. 나 자신(Code 탭)도 앱과 함께 꺼지기 때문이다.
```powershell
$cmd = 'cmd.exe /c ""' + "$kit\install-claude.cmd" + '""'
$r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = $cmd; CurrentDirectory = $kit }
"ReturnValue=$($r.ReturnValue) pid=$($r.ProcessId)"
Start-Sleep 5; Get-Content "$env:TEMP\open-hancom-install.log"
```
- `ReturnValue=0`이고 기록에 `[wait] claude desktop running`이 보이면 예약된 것이다.
- 기록에 바로 `[ok] claude-desktop`이 있으면 앱이 이미 꺼져 있어서 등록이 끝난 것이다.
- Claude Code를 쓰는 PC면 기록에 `[ok] claude-code` 또는 `[fail] claude-code`가 먼저 찍힌다(`CLAUDE_CONFIG_DIR`가 있으면 그 폴더의 `.claude.json`). 이 줄은 Desktop 등록 **전에** 기록되므로 Desktop 결과는 따로 본다.
- Desktop 등록 완료는 기록의 `[ok] claude-desktop`으로만 판단한다. `[fail] claude-desktop`(종료 코드 2), `[timeout]`(3)이면 Desktop은 등록되지 않은 것이다.
- `[fail] claude-code`와 `[ok] claude-desktop`이 둘 다 있으면(종료 코드 4) Desktop은 등록됐고 Claude Code만 실패한 것이다. 각 결과를 그대로 사용자에게 전한다.

## 3. 사용자에게 안내
"이제 Claude를 트레이 아이콘까지 완전히 종료하세요(작업 표시줄 오른쪽 아래 Claude 아이콘 → 종료). 몇 초 뒤 다시 켜면 open-hancom 도구가 붙습니다. 약 15분(300회 확인) 안에 종료하지 않으면 예약이 풀리니, 그때는 설치 폴더의 `install-claude.cmd`를 더블클릭하세요."

## 4. 다시 켠 뒤 확인 (새 대화에서)
- 도구 목록에 `open-hancom`(oh_guide, oh_draft 등)이 보이면 성공이다. `oh_guide`를 한 번 불러 사용 규칙을 읽는다.
- 명령을 쓸 수 있으면: `& "$env:LOCALAPPDATA\OpenHancom\oh.exe" doctor` 결과의 `registered: true`와 `Get-Content "$env:TEMP\open-hancom-install.log"`를 본다.
- 안 보이면 기록 파일의 마지막 JSON의 `error`·`detail`을 사용자에게 그대로 전한다. 자주 나오는 것:
  - `client_running` 상태로 15분 지남(`[timeout]`) → 앱을 트레이까지 끄고 `install-claude.cmd`를 다시 실행.
  - `config_corrupt` → Claude 설정 파일이 깨져 있다. 파일은 건드리지 않았다. 사용자와 같이 확인.

## 설치 결과 위치
- 프로그램: `%LOCALAPPDATA%\OpenHancom\`(oh.exe, library, 개인 라이브러리 `personal-library`)
- 작업 폴더: `문서\OpenHancom\`(inbox에 첨부, outbox에 결과)
- Claude 설정은 바꾸기 전에 `<설정 파일>.oh-backup-<시각>`으로 백업된다.
- 자세한 내용: 같은 폴더의 `INSTALL.md`, 사용 규칙: `PROMPT.md`(=`oh_guide`).
