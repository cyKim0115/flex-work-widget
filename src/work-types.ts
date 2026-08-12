export type WorkPhase = "NeedLogin" | "NotStarted" | "Working" | "Resting" | "Done" | "FetchError";

export type WorkSnapshot = {
  state: WorkPhase;
  workedSeconds: number | null;
  label: string;
  startedAt: string | null;
  error: string | null;
  fetchedAtMs: number | null;
};

export function phaseLabel(state: WorkPhase): string {
  switch (state) {
    case "NotStarted":
      return "출근 전";
    case "Working":
      return "근무 중";
    case "Resting":
      return "휴게 중";
    case "Done":
      return "퇴근";
    case "NeedLogin":
      return "로그인 필요";
    default:
      return "오류";
  }
}

export function tidyMessage(raw: string): string {
  return raw
    .replace(/\r\n/g, "\n")
    .replace(/\nLocation:\s*\n\s*rookie-rs[\s\S]*$/gim, "")
    .replace(/chrome:\s*decrypt_encrypted_value failed/gi, "Chrome 쿠키 복호화 실패")
    .replace(/edge:\s*decrypt_encrypted_value failed/gi, "Edge 쿠키 복호화 실패")
    .replace(/can be decrypted only when running as admin[^\n]*/gi, "관리자 권한(UAC) 필요")
    .trim();
}
