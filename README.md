# BDO Optimizer Launcher

검은사막(BlackDesert64.exe) 실행 + 성능 최적화 + 자동화를 위한 Windows 데스크톱 앱입니다.

기존 Windows 배치 스크립트 기반 런처를 **Rust + Tauri/React UI**로 재구현한 단일 실행 파일(.exe)이며, 시스템 트레이 상주 + 시간표 자동 모드 전환 + PC 예약 종료 + 실시간 자원 모니터링을 한 화면에서 관리합니다.

---

## 요구사항

- **OS**: Windows 10 1809 이상 (Windows 11 권장 — P/E-core 분리 affinity 지원)
- **권한**: 관리자 권한 (UAC `requireAdministrator` 매니페스트 — CPU 우선순위/affinity 조작 필요)
- **CPU**: x64 (32비트 미지원)
- **검은사막**: 검은사막 라이브 클라이언트가 설치되어 있어야 함

---

## 설치 및 실행

### 1. 다운로드
저장소의 **Releases** 페이지에서 최신 `bdo-optimizer-launcher.exe`와 `SHA256SUMS.txt`를 같은 폴더에 다운로드합니다.

### 2. SmartScreen 우회 (코드사이닝 미적용)
배포된 .exe는 코드사이닝 인증서가 적용되지 않아 첫 실행 시 Windows Defender SmartScreen 경고가 표시됩니다.

1. 첫 실행 시 "Windows에서 PC를 보호했습니다" 화면 표시
2. **"추가 정보"** 클릭
3. **"실행"** 버튼 클릭

이후 실행 시에는 경고가 표시되지 않습니다. 실행 전에는 다운로드한 .exe의 SHA-256 해시를 Releases 페이지의 게시 값과 비교해 무결성을 검증할 것을 권장합니다.

PowerShell에서 다운로드 폴더로 이동한 뒤:

```powershell
Get-FileHash .\bdo-optimizer-launcher.exe -Algorithm SHA256
Get-Content .\SHA256SUMS.txt
```

두 출력의 해시 값이 같으면 배포 파일이 변조되지 않은 상태입니다.

### 3. UAC 승인
앱은 관리자 권한이 필요합니다. 첫 실행 시 UAC 프롬프트가 표시되면 **"예"** 선택. 자동 시작을 등록하면(설정 탭) UAC 프롬프트 없이 로그온 시 자동 실행됩니다.

자동 시작은 관리자 권한 작업으로 등록되므로, `.exe`는 `Program Files`처럼 일반 사용자가 임의로 덮어쓰기 어려운 위치에 두는 것을 권장합니다. 사용자 프로필, AppData, 임시 폴더 아래에서 실행 중인 경우 자동 시작 등록이 거부됩니다.

---

## 주요 기능

| 기능 | 설명 |
|---|---|
| **모드 적용** | 고성능(P-core only) / 일반(전체 코어) / 저전력(E-core only 또는 마지막 코어). 게임가드 우회 위해 0.5/1/2/5/10초에 5회 자동 재적용 |
| **자동 모드 전환** | 시간대별(매일/평일/주말/특정 날짜) 자동 모드 전환 규칙 (앱 실행 중에만 동작) |
| **PC 예약 종료** | 단발(다음 1회) 또는 매주 반복(요일 + 시각). Windows 작업 스케줄러 사용 (앱 종료해도 동작) |
| **시스템 트레이 상주** | X 버튼 → 트레이로 숨기기 (설정 토글). 트레이 메뉴에서 창 열기/모드 적용/예약 취소/종료 |
| **자동 트레이/복원** | 게임이 트레이로 내려가면 자동 저전력 모드 + 다시 나타나면 직전 모드 복원 |
| **단일 인스턴스** | Named Mutex로 중복 실행 차단, 두 번째 실행은 기존 창 포그라운드 |
| **자동 시작** | 작업 스케줄러 로그온 트리거(UAC 프롬프트 없이 승격). 사용자 쓰기 가능성이 높은 위치의 .exe는 등록 거부. 트레이 시작 옵션 가능 |
| **업데이트 알림** | 설정 탭에서 GitHub Release 최신 버전을 확인하고, 새 버전이 있으면 릴리스 페이지를 엽니다. 실행 파일 자동 교체는 하지 않습니다 |
| **자원 모니터링** | 검은사막 프로세스의 CPU/메모리/GPU/VRAM/FPS 실시간 + 코어별 사용률 |
| **테마** | 라이트/다크/Windows 자동 (OS 설정 추종) |
| **접근성** | Tab 키 순회, Ctrl+1~4 탭 단축키, ESC 캘린더 dismiss, 주요 커스텀 컨트롤의 키보드 활성화 |

---

## P/E-core (Alder Lake+ Intel) 지원

12세대 이후 Intel CPU(13700K, 14900K 등)는 P-core(성능)와 E-core(효율)가 혼합되어 있습니다. 본 앱은 `GetLogicalProcessorInformationEx` + `EfficiencyClass`로 자동 인식합니다.

- **고성능 모드**: P-core 전체 thread (E-core 차단) — 검은사막은 단일 코어 throughput 의존이라 P-core only가 권장됩니다.
- **저전력 모드**: E-core 전체 (P-core 차단) — 백그라운드 실행 시 발열·소비전력 ↓.
- **일반 모드**: 전체 코어 (변경 없음).

AMD CPU 및 구형 Intel CPU(11세대 이하)는 기존 로직(짝수 비트/전체/마지막 비트)을 그대로 사용합니다.

---

## 문제 해결

### 모드 적용 후 priority가 원래대로 돌아옴
검은사막 게임가드(XignCode3)가 외부 priority 조작을 reset할 수 있습니다. 본 앱은 0.5/1/2/5/10초에 5회 자동 재적용으로 대응하지만, 일부 환경에서는 완전 차단됩니다. 이 경우 본 앱이 동작하지 않습니다.

### "관리자 권한이 없어 최적화를 적용할 수 없습니다"
앱을 우클릭 → **"관리자로 실행"** 선택. 자동 시작 등록을 활성화하면 다음 부팅부터 UAC 프롬프트 없이 자동 승격됩니다.

### 자동 시작이 동작하지 않음
설정 탭 → "Windows 시작 시 자동 실행"을 한 번 OFF → ON 토글하여 작업 스케줄러에 재등록. `schtasks /query /tn BDO_Optimizer_Launcher_Autostart`로 작업 등록 여부 확인.

사용자 폴더, AppData, 임시 폴더 아래에서 실행 중이면 보안상 자동 시작 등록이 거부됩니다. `.exe`를 `C:\Program Files\BDO Optimizer\` 같은 관리자 전용 위치로 옮긴 뒤 다시 등록하세요.

### 모니터 탭의 FPS가 "측정 중..." 또는 0으로 표시됨
- ETW(이벤트 추적 윈도우) 세션 초기화 실패 가능성. 관리자 권한 확인.
- NVIDIA Reflex, Hardware GPU Scheduling, G-Sync가 활성화된 환경에서 DXGI PresentStart 경로가 우회될 수 있습니다.
- 다른 ETW 도구(PresentMon 등)가 동시 실행 중이면 세션 충돌. 다른 도구 종료 후 재시도.

### 폰트가 깨져 보임
나눔고딕이 .exe에 임베드되어 있어 별도 폰트 설치 없이 동작합니다. 그래도 일부 글리프(이모지)는 OS fallback에 의존합니다.

### 트레이 아이콘이 표시되지 않음
시스템 트레이 알림 영역에서 "숨겨진 아이콘 표시"를 확인. Windows 설정 → 개인 설정 → 작업 표시줄에서 본 앱 아이콘을 "항상 표시"로 설정 권장.

### 로그 파일 위치 (문제 보고 시 첨부)
앱이 동작하면서 발생한 이벤트와 오류는 다음 위치에 일일 파일로 기록됩니다:

```
%LOCALAPPDATA%\bdo-optimizer-launcher\logs\bdo-optimizer.YYYY-MM-DD
```

예: `C:\Users\<사용자>\AppData\Local\bdo-optimizer-launcher\logs\bdo-optimizer.2026-05-24`

기본 로그 레벨은 `INFO`이며, 더 자세한 로그가 필요하면 환경변수로 조정 가능:
```
set RUST_LOG=debug
bdo-optimizer-launcher.exe
```

문제 보고 시 해당 날짜의 로그 파일을 첨부해 주시면 진단에 도움이 됩니다.

---

## ⚠️ 면책 / 사용 위험

본 도구는 검은사막의 priority 및 CPU affinity를 외부에서 조작합니다. 이는 검은사막 ToS(서비스 약관)의 회색지대에 해당하며, **Pearl Abyss 게임가드(XignCode3)가 외부 최적화 도구로 인식해 차단하거나, 더 나아가 계정 제재로 이어질 가능성을 완전히 배제할 수 없습니다.** 본 도구의 사용은 전적으로 사용자 본인의 책임이며, 개발자는 사용으로 인한 어떠한 결과에도 책임을 지지 않습니다.

또한 본 도구는 다음 시스템 권한을 사용합니다:
- 관리자 권한 (UAC requireAdministrator)
- 다른 프로세스의 CPU priority/affinity 변경 (`SetPriorityClass`, `SetProcessAffinityMask`)
- ETW(Event Tracing for Windows) DXGI provider 구독 (FPS 측정)
- Windows 작업 스케줄러 등록/해제 (자동 시작, PC 예약 종료)
- Windows 트레이 아이콘 등록
- 레지스트리 읽기 (OS 테마, CPU/GPU 정보)

---

## 개발자 정보

- **언어**: Rust (edition 2021) + Tauri v2 + React/Vite
- **빌드**: `npm run build` 후 `cargo build --release --locked`
- **테스트**: `cargo test --all-targets --locked`
- **검증**: `cargo fmt --all -- --check` / `cargo clippy --all-targets --no-deps --locked -- -D warnings` / `npm run check:design-parity` / `npm run build`
- **CI**: GitHub Actions (`.github/workflows/ci.yml`, `release.yml`)
- **라이센스**: 본 저장소 LICENSE 파일 참고. 임베드 폰트(나눔고딕)는 SIL Open Font License 1.1.

---

## 변경 이력

마일스톤별 변경 이력은 `.ai/memory/plan.md` (개발자용)를 참고하세요.
