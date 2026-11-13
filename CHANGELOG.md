# 변경 기록

버전 규칙
- 배포 묶음과 코어(`oh.exe`)는 같은 번호를 쓴다(workspace `version`). zip 이름도 이 번호다.
- 라이브러리는 `library/manifest.toml`의 `version`으로 따로 센다.
- 카드의 `requires_core`는 그 카드가 쓰는 코어 기능의 최소 버전이다. 코어가 이보다 낮으면 그 카드는 `core_too_old`로 거부된다.
- 1.x 안에서는 카드·틀 TOML 형식, MCP 도구 이름·인수, 설치 폴더 구조(`library/`, `personal-library/`, `sockets.toml`)를 깨지 않는다. 깨야 하면 2.0으로 올린다.
- 재설치·업데이트는 `personal-library/`, `sockets.toml`, 작업 폴더(`문서\OpenHancom`)를 건드리지 않는다.

## 1.1.0 (2026-11-13)
팩 만들기 도구를 `oh`에 넣었다. 공용 팩이나 기본 라이브러리에 새 양식을 넣을 때 Python 스크립트 없이 Windows에서 바로 한다.

추가
- `oh library anonymize`: 치환표로 HWPX를 익명화한다. 작성자 정보, 미리보기 이미지, 줄 배치 캐시까지 정리하고, 원본은 바꾸지 않는다(`scripts/anon-dept.py` 대체).
- `oh library scan`: 검사어·이메일·전화·주민번호·미리보기 이미지 잔존 스캔. 파일, 폴더, zip, HWPX·XLSX 멤버까지 본다. 검사어는 줄 번호로만 보고한다(`scripts/anon-scan.py` 대체, `dist-scan.py` 규칙).
- `oh library manifest`: 팩 manifest를 해시와 함께 다시 쓴다. 스캔이 0일 때만 `anonymized = true`(`scripts/library-manifest.py` 대체, 기본 라이브러리에서 같은 바이트).
- `oh library check`: 연결하기 전에 `add`와 같은 검사를 한다. 기록은 하지 않는다.

바뀜
- `oh library add|list|remove`가 쓰지 않는 옵션을 주면 거부한다(`invalid_arguments`).
- 라이브러리·카드 형식, MCP 도구, 설치 폴더 구조는 1.0.0과 같다. 1.0.0 위에 그대로 덮어 설치하면 된다.

알려진 한계
- 익명화는 한 글자 덩어리(run) 안의 문자열만 바꾼다. 여러 조각으로 나뉜 원문과 내장 이미지는 결과를 `oh library scan`으로 다시 보고 손으로 확인한다.

## 1.0.0 (2026-11-12)
처음으로 동료 공유와 GitHub 공개를 하는 버전이다(D14).

추가
- 등록 흐름: 사용자 HWPX를 카드로 만든다(`oh_library_register` → 사용자 확인 → `oh_library_commit`). 만든 카드는 개인 라이브러리에 저장된다.
- 일반 품의서 카드(`general-approval`).
- 번호 중복 검사(범용): 한컴 자동 번호와 글자 번호가 겹치면 막는다(`numbering_doubled`). 날짜일 수도 있으면 경고만 한다(`numbering_ambiguous`). 자동 번호 문단이 "1. 2."처럼 날짜 모양으로 시작하는 경우도 경고한다.
- 배포 zip 두 종류: 내부용(카드 8종)과 공개용(공문 카드 1종). 제3자 고지와 Rust 표준 라이브러리 고지를 동봉한다.
- ChatGPT 데스크톱용 로컬 설치 키트(`kit/chatgpt`, 저장소에만 있음).
- Claude 자가 설치 키트: `install-claude.cmd`(Claude 앱이 꺼지길 기다렸다가 등록)와 `CLAUDE-SETUP.md`(Claude가 읽고 설치를 진행하는 절차).

바뀜
- 릴리스 빌드: LTO, strip, Windows crt-static(깨끗한 Windows에서 DLL 없이 실행).
- 라이선스: Apache-2.0.
- `oh_calibrate` 도구 제거.
- 라이브러리 템플릿·예시에서 자동 번호 문단의 글자 번호 제거(라이브러리 0.8.2).
- 라이브러리 0.8.3: 업체등록의 부서·첨부를 입력값으로(없으면 빈칸), 시작일을 체결일로 단정하는 문구 제거, 출장신청 하단 줄 간격 조정.

알려진 한계
- `oh.exe`는 코드 서명이 없다. 처음 실행할 때 Windows 경고가 뜰 수 있다(INSTALL 참조).
- 실행 중인 Claude Desktop은 설정을 다시 써서 등록을 지울 수 있다. 앱을 트레이까지 끈 뒤 등록한다(키트가 기다려 준다).
- 배포 스캔은 실행 파일·압축 데이터 안의 이메일 주소를 좁게 잡는다.

## 0.5.0 이전 (내부 개발)
- 0.5.0: 1.0 1단계(등록 흐름, 일반 품의서, 라이브러리 0.8.x).
- 0.4.0: 공문 틀(`official-letter`)과 발주기관 공문 카드, 번호 필터(num/ganada/attach), 공용 라이브러리 소켓(개인 > 팩 > 기본), 설치 가드(`client_running`, `verified`).
- 0.3.0: 품의 공통 틀(`approval-memo`), 보험·보증 카드, 사업 기록, 계산 함수 확장.
- 그 전: 카드 엔진, 외주업체 등록·하도급 승인·해외출장 신청·정산 카드, MCP 서버와 `oh install/uninstall/doctor`, OS 계층 분리(`oh-host`).
