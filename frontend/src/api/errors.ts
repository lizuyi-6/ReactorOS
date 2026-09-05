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

// 后端校验/业务错误里的字段名 → 中文，用于把动态报文翻译完整
const FIELD_LABELS: Record<string, string> = {
  target_temperature_c: "目标温度",
  target_stirrer_rpm: "目标转速",
  target_shake_speed_cpm: "摇晃速度",
  target_pressure_mpa: "目标压力",
  heat_time_s: "加热时长(秒)",
  hold_time_s: "保温时长(秒)",
  cool_time_s: "冷却时长(秒)",
  temperature_c: "温度",
  stirrer_rpm: "转速",
  yield_percent: "收率",
  product_ratio: "产品比例"
};

function fieldLabel(field: string): string {
  return FIELD_LABELS[field] ? tr(FIELD_LABELS[field], field) : field;
}

// V20 修复：后端固定英文错误串在 UI 层本地化（中文界面不再弹英文）
// 回调接收正则匹配结果，支持动态字段（如 "{field} must be between {min} and {max}"）。
const KNOWN_MESSAGE_MAP: Array<[RegExp, (m: RegExpMatchArray) => string]> = [
  [/invalid username or password/i, () => tr("用户名或密码错误", "Invalid username or password")],
  [/account is locked/i, () => tr("账户已锁定，请稍后再试", "Account locked, try later")],
  [/current password is incorrect/i, () => tr("当前密码不正确", "Current password is incorrect")],
  [/new password must be at least (\d+) characters/i, (m) => tr(`新密码长度至少 ${m[1]} 位`, `New password must be at least ${m[1]} characters`)],
  [/new password must differ from the current password/i, () => tr("新密码不能与当前密码相同", "New password must differ from the current password")],
  [
    /batch start must include at least one explicit target or duration field/i,
    () => tr("启动批次失败：工艺缺少有效的目标值或时长字段，请先在配方中完善", "Batch start requires at least one explicit target or duration field")
  ],
  [
    /(\w+) must be between ([-\d.]+) and ([-\d.]+)/i,
    (m) => tr(`${fieldLabel(m[1])} 必须在 ${m[2]} 到 ${m[3]} 之间`, `${fieldLabel(m[1])} must be between ${m[2]} and ${m[3]}`)
  ],
  [/(\w+) is required/i, (m) => tr(`${fieldLabel(m[1])} 为必填项`, `${fieldLabel(m[1])} is required`)],
  [
    /target pair .* enters forbidden control zone ([^:]+):\s*(.*)/i,
    (m) => tr(`目标温度/转速组合进入禁止控制区「${m[1].trim()}」：${m[2].trim()}`, `Target pair enters forbidden control zone ${m[1].trim()}: ${m[2].trim()}`)
  ],
  [
    /role '(\w+)' lacks permission '(\w+)'/i,
    (m) => tr(`当前角色（${m[1]}）没有该操作的权限（${m[2]}）`, `Role '${m[1]}' lacks permission '${m[2]}'`)
  ],
  [/register is not writable through the Modbus debug API/i, () => tr("该寄存器不支持通过 Modbus 调试接口写入", "Register is not writable through the Modbus debug API")],
  [/modbus debug writes require admin role/i, () => tr("Modbus 调试写入需要 admin 角色", "Modbus debug writes require the admin role")],
  [/missing bearer session token|invalid bearer session|bearer session has expired/i, () => tr("登录状态已失效，请重新登录", "Session expired, please sign in again")]
];

export function errorMessage(error: unknown, fallback = tr("请求失败", "Request failed")): string {
  const raw = toApiError(error).message || fallback;
  for (const [re, local] of KNOWN_MESSAGE_MAP) {
    const match = raw.match(re);
    if (match) return local(match);
  }
  return raw;
}
