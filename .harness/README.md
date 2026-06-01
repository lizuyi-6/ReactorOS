# ReactorOS Harness

Mavis 给 ReactorOS (ReactorOS Edge Supervisor) 项目组建的持久化开发团队。

## 结构

```
.harness/
├── agent.md                       # Harness orchestrator — 路由任务到 5 个 reins
├── docs/
│   ├── project-overview.md        # 速读导览 + 硬约束
│   └── coding-standards.md        # 各模块代码规约
├── reins/
│   ├── reactor-daemon-expert/     # Rust 后端
│   ├── reactor-firmware-expert/   # ESP32 固件
│   ├── reactor-qt-expert/         # Qt HMI
│   ├── reactor-hmi-expert/        # Web + E2E + kiosk
│   └── reactor-build-expert/      # 构建/部署
├── memory/
│   └── MEMORY.md                  # 跨 reins 共享记忆
└── changelogs/                    # 预留给每日提交日志
```

## 怎么用

- 进来一个 ReactorOS 任务,看根 `agent.md` 路由表判断归哪个 rein
- 单文件小改 → 直接派对应 rein
- 跨模块大改 → 拆给 2+ reins,各自 Own 自己那块
- reins 干活前先读 `docs/project-overview.md` + `docs/coding-standards.md`

## 怎么扩展

- 加新 rein:`mavis agent new <name> --target project --project X:\tianhks`
- 改 reins:直接编辑 `.harness/reins/<name>/agent.md`
- 改 orchestrator:编辑 `agent.md` (但**别在里面手写 reins 列表** — daemon 会自动注入)

## git

`.harness/` 是项目资产,**应该** commit 进去:`git add .harness/ && git commit -m "init harness: ReactorOS dev team"`。
