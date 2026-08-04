# Flex Work Widget

Windows 바탕화면에 띄워 두는 Flex 오늘 누적 근무시간 위젯입니다.  
`cursor-usage-widget`와 같은 Tauri 2 데스크톱 위젯 형태입니다.

## 범위 (MVP)

- 오늘 **누적 근무시간** 표시 (`HH:MM:SS`)
- **출근 전**이면 그 상태 표시
- 근무 중 / 휴게 중 / 퇴근 상태 구분
- flex 웹 로그인 세션 사용 (Open API 토큰 불필요)

## 데이터 경로

비공식 웹 세션 방식입니다.

1. 위젯에서 **flex 로그인** 창을 엽니다 (`https://flex.team`)
2. 로그인 후 웹뷰 세션/쿠키로 다음 API를 호출합니다  
   - `GET /api/v2/workspace/users/me/workspace-users`  
   - `GET /api/v2/time-tracking/work-clock/users/{userIdHash}/current-status`
3. 응답의 근무/휴게 블럭으로 오늘 누적 시간을 계산합니다

flex 웹 구조가 바뀌면 깨질 수 있습니다.

## 실행

일상 사용: `시작.bat`  
→ 릴리즈 exe를 `%LOCALAPPDATA%\FlexWorkWidget\`에 두고 실행합니다. 커맨드창은 남지 않습니다.  
→ 처음 한 번은 빌드가 필요할 수 있습니다 (`npm run build:app`).

개발(핫 리로드): `시작-개발.bat` 또는

```powershell
cd C:\Users\cykim\repo\flex-work-widget
npm install
npm run tauri dev
```

### 첫 연결

앱 내 로그인 창은 flex(특히 Google 로그인)에서 **하얀 화면**이 나므로 사용하지 않습니다.

1. 위젯 우클릭 → **1. 브라우저에서 로그인** (Chrome/Edge)
2. flex 홈까지 로그인 완료
3. 위젯 우클릭 → **2. 세션 가져오기**  
   - Chrome/Edge **모든 프로필**을 최근 사용 순으로 스캔합니다
   - Chrome v130+ 쿠키 암호 때문에 **관리자 권한(UAC)** 확인이 뜹니다 → 허용
   - Chrome만 로그인돼 있고 복호화가 막히면 Edge에서 flex.team에 로그인한 뒤 다시 시도하세요

의존성: `pip install rookiepy` (세션 가져오기 스크립트용)

## 표시 모드

우클릭 메뉴 또는 시간 숫자 클릭으로 전환합니다.

- **누적 근무시간**: 오늘 근무한 시간
- **남은 근무시간**: `8시간 − 누적` (0 미만이면 00:00:00)

## 테마

우클릭 메뉴에서 **표시 모드** / **테마**를 펼쳐 선택합니다. 기본 테마는 **시스템**입니다.

우클릭 → **종료**로 앱을 닫습니다.

## 개발 메모

- 세션 파일: `%LOCALAPPDATA%\FlexWorkWidget\session.json` (커밋 금지)
- 웹뷰 데이터: `%LOCALAPPDATA%\FlexWorkWidget\webview\`
- 폴링 기본 간격: 60초 (근무 중에는 UI에서 1초 단위로 로컬 틱)
- Windows WebView2 `cookies_for_url`로 HttpOnly 쿠키까지 읽습니다
