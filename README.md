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

```powershell
cd C:\Users\cykim\repo\flex-work-widget
npm install
npm run tauri dev
```

또는 `시작.bat`

### 첫 연결

1. 위젯 우클릭 → **앱에서 로그인**
2. 가능하면 **이메일 로그인** 사용  
   (Google 로그인은 WebView2에서 하얀 화면이 날 수 있습니다)
3. flex 홈까지 들어가면 세션을 자동으로 가져옵니다  
   (안 되면 **세션 가져오기**)

Google만 되는 계정이면 **시스템 브라우저로 로그인**으로 flex 자체는 열고, 위젯 연동은 앱 로그인 창에서 다시 시도해 주세요.

## 개발 메모

- 세션 파일: `%LOCALAPPDATA%\FlexWorkWidget\session.json` (커밋 금지)
- 웹뷰 데이터: `%LOCALAPPDATA%\FlexWorkWidget\webview\`
- 폴링 기본 간격: 60초 (근무 중에는 UI에서 1초 단위로 로컬 틱)
- Windows WebView2 `cookies_for_url`로 HttpOnly 쿠키까지 읽습니다
