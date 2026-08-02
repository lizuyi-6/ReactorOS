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

export function errorMessage(error: unknown, fallback = "请求失败"): string {
  return toApiError(error).message || fallback;
}
