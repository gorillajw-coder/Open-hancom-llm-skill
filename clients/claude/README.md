# Claude

Windows 공개 릴리스 ZIP 전체를 풀고 Claude Desktop을 **트레이까지 종료**한 뒤 `clients\claude\install.cmd`를 실행합니다. 사용자가 앱을 다시 열고 `oh_guide`를 호출해야 실제 연결이 확인됩니다. 이전 배포의 `install-claude.cmd`와 `CLAUDE-SETUP.md`도 계속 사용할 수 있습니다.

직접 등록하거나 Claude Code에 연결하려면 배포 폴더의 PowerShell에서 실행합니다.

```powershell
.\oh.exe install --client claude-desktop
# Claude Code를 쓸 때만 별도로:
.\oh.exe install --client claude-code
```

기본 작업 폴더는 문서 폴더의 `OpenHancom`입니다. 원본은 `inbox`, 결과는 `outbox`를 사용합니다. 관리자 권한은 필요하지 않습니다. 보안 제품이 실행을 거부하면 보안 담당자에게 확인하고 우회하지 않습니다.

첫 요청: “open-hancom의 oh_guide를 읽고 공문 예시 카드를 확인해 줘. 문서는 한국어로 작성하고, 빠진 사실은 물어봐. 계산과 파일 생성은 코어 도구로 처리해.”
