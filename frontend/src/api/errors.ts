import { tr } from "../i18n";

export interface ApiError {
  status: number;
  message: string;
  payload?: unknown;
}

export function isApiError(error: unknown): error is ApiError {
  return Boolean(error && typeof error === "object" && "status" in (error as ApiError) && "message" in (error as ApiError));
}

export function toApiError(error: unknown): ApiError {
  if (isApiError(error)) return error;
  if (error instanceof Error) return { status: 0, message: error.message };
  return { status: 0, message: String(error) };
}

// V20 修复：后端固定英文错误串在 UI 层本地化（中文界面不再弹英文）
const KNOWN_MESSAGE_MAP: Array<[RegExp, () => string]> = [
  [/invalid username or password/i, () => tr("用户名或密码错误", "Invalid username or password")],
  [/account is locked/i, () => tr("账户已锁定，请稍后再试", "Account locked, try later")]
];

export function errorMessage(error: unknown, fallback = tr("请求失败", "Request failed")): string {
  const raw = toApiError(error).message || fallback;
  for (const [re, local] of KNOWN_MESSAGE_MAP) {
    if (re.test(raw)) return local();
  }
  return raw;
}
