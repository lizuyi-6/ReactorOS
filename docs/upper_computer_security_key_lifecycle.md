# 星宿上位机密钥生命周期与敏感字段清单

日期：2026-06-04

范围：李祖祎负责的上位机软件。本文档把当前代码已经实现的密钥、证书、token、敏感字段和生产验收要求固化成检查清单；它不是正式渗透测试或等保报告。

## 1. 当前密钥与凭据清单

| 项 | 配置位置 | 当前用途 | 当前代码状态 | 生产要求 |
| --- | --- | --- | --- | --- |
| `XINGSHU_DB_ENCRYPTION_KEY` | 环境变量 | SQLite 集成任务请求/回执 AES-256-GCM 静态加密 | 已实现；支持 32 字节原文、64 位 hex 或 base64；`/api/config/summary.data_security.storage_encryption` 可见 | 必须由生产密钥管理系统生成、备份、分发和轮换；不得写入仓库 |
| `XINGSHU_AUTH_SECRET` | 环境变量 | 本地 bearer session 签名 | 已实现；未设置时使用本地默认开发 secret | 生产必须覆盖；轮换后所有旧 session 失效 |
| `XINGSHU_OPERATOR_PASSWORD` / `XINGSHU_ENGINEER_PASSWORD` / `XINGSHU_ADMIN_PASSWORD` | 环境变量 | 默认本地账号密码覆盖 | 已实现；未设置时使用本地演示密码 | 生产必须覆盖，且按角色最小权限发放 |
| HTTP TLS `--tls-cert` / `--tls-key` | daemon 启动参数 | HTTPS 入口证书和私钥 | 已实现；必须成对提供 | 使用正式 CA 或企业 CA，私钥只允许服务账号读取 |
| MQTT `ca_cert` / `client_cert` / `client_key` | `config/integration.toml` | MQTT broker CA 与客户端证书 | 已实现配置解析、本地状态摘要和 `use_tls=true` 缺少非空 `ca_cert` 时 fail-closed | 生产 broker 证书链、客户端证书和私钥需外部 broker 验收 |
| MQTT `username` / `password` | `config/integration.toml` | MQTT broker 用户认证 | 已实现字段和 rumqttc credentials 设置 | 不得提交真实账号；建议通过部署模板注入 |
| Modbus TCP `tls_cert` / `tls_key` | `config/integration.toml` | Modbus TCP TLS server 证书与私钥 | 已实现；本地 TLS/MBAP 回归通过 | 生产证书链、外部 Modbus Poll/Slave TLS 需验收 |
| `STEPFUN_API_KEY` | 环境变量 | 云端大模型 API bearer key | 已实现 provider 调用 | 不得落库或进入日志；生产需最小权限 key 和调用审计 |
| `XINGSHU_TOKEN` | 环境变量或 CLI `--token` | CLI bearer session token | 已实现 | 只作为短期 session 使用，不写入脚本和文档 |
| 本地 AI 资产路径 | `XINGSHU_LOCAL_AI_BIN`、`XINGSHU_LOCAL_AI_GGUF`、`XINGSHU_LOCAL_AI_LORA`、`XINGSHU_LOCAL_AI_TRAIN_SCRIPT`、`XINGSHU_LOCAL_AI_CONVERT_SCRIPT`、`XINGSHU_LOCAL_AI_RK_REPORT` | 本地 Qwen/LoRA readiness 边界 | 只做 readiness 检查；真实推理/训练未实现 | 模型权重、LoRA adapter、训练数据和报告按算法资产管理 |

## 2. 已加密字段清单

`src/db.rs` 当前只对第三方集成任务的原始请求和回执做 AES-256-GCM 信封加密：

| SQLite 表 | 字段 | 加密状态 | 说明 |
| --- | --- | --- | --- |
| `integration_tasks` | `request_json` | 已支持 | 存放 AINAS/MQTT 任务载荷；启用 `XINGSHU_DB_ENCRYPTION_KEY` 后以 `xingshu:v1:aes256gcm:` 前缀信封写入 |
| `integration_tasks` | `response_json` | 已支持 | 存放任务执行回执；同上 |

以下字段当前不做数据库字段级加密，需通过主机磁盘加密、备份权限和日志策略保护：

| 数据类别 | 存储位置 | 原因/处置 |
| --- | --- | --- |
| 传感器样本、批次、产物结果 | SQLite 普通表 | 属于实验数据；如含商业敏感实验配方，生产需启用磁盘加密和备份访问控制 |
| 审计事件 | SQLite 普通表 + hash chain | 需要可检索和可验证；生产需备份和防删除策略 |
| 进程/工艺定义 | SQLite 普通表 | 生产需确认是否包含商业配方，必要时扩展字段级加密 |
| TLS/MQTT 私钥 | 文件系统路径 | 不进入 SQLite；生产通过文件权限和密钥管理系统保护 |
| bearer session token | 客户端持有 | 不落库；过期时间 12 小时 |

## 3. 密钥生成、备份和轮换流程

1. 生成：使用生产密钥管理系统或离线随机源生成 32 字节密钥。验收可用 64 位 hex 或 base64 表示。
2. 分发：通过部署环境变量注入 `XINGSHU_DB_ENCRYPTION_KEY`、`XINGSHU_AUTH_SECRET` 和角色密码，不写入 Git、SQLite、截图或报告。
3. 备份：`XINGSHU_DB_ENCRYPTION_KEY` 必须与数据库备份成对托管。丢失密钥后，已加密的 `integration_tasks.request_json/response_json` 无法恢复。
4. 轮换：当前代码支持新密钥加密新写入和同密钥读取旧密文，但尚未提供自动重加密迁移工具。生产轮换需执行停机、旧密钥导出、重加密迁移、恢复验证和旧密钥封存。
5. 吊销：轮换 `XINGSHU_AUTH_SECRET` 会使旧 bearer token 全部失效。证书吊销需同步 broker、Modbus TCP 客户端和 HTTP 入口。

## 4. 验收检查项

| 检查项 | 本地状态 | 仍需生产验收 |
| --- | --- | --- |
| `XINGSHU_DB_ENCRYPTION_KEY` 开启后原始 SQLite 不含 AINAS/MQTT 请求/回执明文 | 已由 `db_tests` 覆盖 | 用生产 key 重跑数据库备份/恢复 |
| `/api/config/summary` 暴露加密状态和字段清单 | 已实现 | 运维监控接入 |
| 默认密码可被环境变量覆盖 | 已实现 | 生产密码策略和交接记录 |
| TLS 证书/私钥成对校验 | 已实现 | 正式证书链、私钥权限和吊销演练 |
| MQTT/Modbus TCP 证书链 | 配置和本地测试已有 | 外部 MQTT.fx、Modbus Poll/Slave 验收 |
| 密钥轮换自动化 | 未实现 | 需补重加密迁移工具或明确人工 SOP |
| 正式漏洞扫描/渗透测试 | 未执行 | 需安全负责人出具报告 |

## 5. 当前结论

上位机已经具备 RBAC bearer session、HTTP/Modbus TCP TLS、本地 AES-256-GCM 字段级加密、审计 hash chain 和配置状态可视化。生产交付前仍必须补齐真实密钥托管、轮换演练、证书链外部验收、敏感实验数据分级和正式安全扫描报告。
