# A/B OTA 与工业硬件协同讨论纪要

本文整理了关于工业设备 OTA、A/B 更新、fallback/recovery 以及 PCB 硬件配合的讨论，便于软件、硬件和现场交付伙伴统一理解。

## 1. A/B 更新是什么

A/B 更新来自 Android 的 Seamless Update 思路。设备保留两个可运行的软件槽位，当前运行一个槽位，更新写入另一个槽位。

```text
当前运行：Slot A
后台更新：Slot B

更新完成后：
下次启动尝试 Slot B

如果 Slot B 启动成功：
标记 B 为成功，后续使用 B

如果 Slot B 启动失败：
自动回到 Slot A
```

它的核心不是“两套硬件”，而是：

```text
更新时不破坏当前可用版本
新版本必须通过健康检查才算成功
失败后设备必须能自动恢复到旧版本
```

本项目的软件侧还把危险绕过做成显式双参数确认：无校验包必须同时使用 `--allow-missing-checksum --confirm-unsafe-no-checksum`，跳过数据库备份必须同时使用 `--skip-backup --confirm-skip-backup`，维护窗口强制升级或回滚必须同时使用 `--force --confirm-maintenance-window`。默认 SOP 不使用这些绕过。

## 2. 对本项目的建议

本项目建议先做应用级 A/B，而不是一开始就做完整系统级 A/B。

应用级 A/B：

```text
/opt/reactor-edge/slots/a
/opt/reactor-edge/slots/b
/opt/reactor-edge/current -> slots/a 或 slots/b
```

共享配置和数据：

```text
/etc/reactor-edge/          配置
/var/lib/reactor-edge/      SQLite 数据
/project/                   state.json / control.json
```

适合更新：

```text
reactor-edge-daemon
reactor-safety-guard
Vue HMI
脚本
systemd unit
```

系统级 A/B 适合后续再考虑：

```text
boot_a
boot_b
rootfs_a
rootfs_b
data
recovery
```

系统级 A/B 主要用于更新 Linux rootfs、内核、驱动、系统库，成本更高，需要 bootloader、分区表、镜像构建、启动计数和回滚标记配合。

## 3. Google Virtual A/B 的理解

Google 后来提出了 Virtual A/B，也就是虚拟 A/B。它不是取消 A/B，而是用 snapshot / COW 快照机制减少完整双份系统分区的空间占用。

传统 A/B：

```text
boot_a
boot_b
system_a
system_b
vendor_a
vendor_b
product_a
product_b
data
```

Virtual A/B：

```text
boot_a
boot_b
super 分区
  system
  vendor
  product

OTA snapshot / COW 临时快照
data
```

优点：

- 省空间。
- 仍支持失败回滚。
- 更新过程可以后台准备。
- 适合系统分区很大的 Android 设备。

代价：

- 需要 dynamic partitions。
- 需要 device-mapper。
- 需要 snapshot / COW。
- 需要 snapuserd。
- 需要 bootloader slot 状态。
- 需要 AVB / verified boot 配合。

对本项目来说，Virtual A/B 的思想可以借鉴，但不建议一开始完整照搬 Android 方案。更务实的路线是：

```text
第一阶段：应用级 A/B OTA
第二阶段：rootfs 人工镜像升级或传统 rootfs A/B
第三阶段：设备量大后，再考虑 RAUC / Mender / SWUpdate / OSTree / 类 Virtual A/B
```

## 4. 1.0 程序是否可以作为 fallback

可以，但更建议把它设计成 factory fallback / recovery image / golden image。

不要把“焊死的 1.0 程序”设计成长期参与生产控制的老版本。更安全的做法是：

```text
Fallback/recovery 负责救援，不负责继续生产控制
```

推荐结构：

```text
SPI NOR Flash
  - U-Boot / bootloader
  - boot 配置
  - recovery 启动逻辑

eMMC
  - app_slot_a
  - app_slot_b
  - data
  - recovery
  - factory_package / golden image
```

fallback 触发场景：

```text
A 槽启动失败
B 槽启动失败
连续 N 次健康检查失败
用户按住 Recovery 按键开机
OTA 状态异常
```

进入 fallback 后做这些事：

```text
停止自动控制
只开最小 HMI / SSH / Web 恢复页
允许从 U 盘、以太网、内置 factory_package 重新安装
保留现场数据，不随便清空 /var/lib
```

## 5. PCB 硬件清单

### 5.1 核心必选

| 模块 | 建议规格 | 用途 |
|---|---|---|
| 工业级 eMMC | 16GB 最低，推荐 32GB | 存 A/B 两套程序、数据、日志、恢复包 |
| SPI NOR Flash | QSPI，16MB-64MB | 放 bootloader、recovery 引导、启动状态 |
| 硬件 Watchdog / Supervisor | 独立芯片，能拉 RESET | 新版本卡死时自动复位 |
| 电压监控 Brown-out Reset | 监控主电源和核心电压 | 防止低压写坏存储 |
| 掉电检测 | 24V/12V 输入掉电中断给 CPU | OTA/数据库写入时提前收尾 |
| Recovery 按键/拨码 | 1 个按键或 2 位 DIP | 强制进 recovery / 旧版本 |
| UART 调试口 | 3.3V TTL，建议 4pin | 现场救砖、看 bootloader 日志 |
| 以太网 | 10/100 或千兆，带保护 | OTA 主通道，比 Wi-Fi 稳 |
| 状态 LED | RUN / UPDATE / FAIL / RECOVERY | 现场不用接屏也能判断状态 |

### 5.2 强烈建议

| 模块 | 建议规格 | 用途 |
|---|---|---|
| USB Host | 至少 1 路 USB-A 或 Type-C Host | U 盘离线升级/恢复 |
| RTC | 带电池或超级电容 | 断网后仍能记录故障/升级时间 |
| FRAM / EEPROM | I2C，FRAM 优先 | 保存当前 slot、OTA 状态、失败原因 |
| 保持电容 / 小 UPS 接口 | 按关机/落盘时间计算 | 防更新关键阶段瞬断 |
| JTAG/SWD 测试点 | 预留焊盘即可 | 深度调试和量产救援 |
| Boot mode 测试点 | SoC 启动模式脚预留 | eMMC/SPI 启动异常时救援 |

### 5.3 工业防护

| 模块 | 用途 |
|---|---|
| 24V 输入 TVS、保险丝、反接保护 | 防浪涌、短路、误接 |
| 电源浪涌/ESD 保护 | 抗现场电源干扰 |
| 以太网口 TVS/共模电感/隔离变压器 | 抗浪涌、ESD、共模干扰 |
| RS485/CAN 隔离 | 现场总线强干扰隔离 |
| USB 口 ESD 保护 | 防静电损伤 |
| 存储电源独立去耦和稳定供电 | 降低 eMMC/SPI NOR 写入风险 |

### 5.4 最小可行 BOM

```text
32GB 工业级 eMMC
32MB SPI NOR
硬件 watchdog/supervisor
掉电检测
brown-out reset
Recovery 按键/拨码
UART 调试口
以太网
USB Host
RUN/UPDATE/FAIL/RECOVERY LED
```

## 6. 各模块作用解释

### 6.1 工业级 eMMC

eMMC 是主存储，类似板载硬盘。它负责存 Linux 系统、应用程序、HMI、日志、数据库、A/B 两套版本。

工业场景里普通 TF 卡容易因为断电、频繁写日志、高温、震动接触不良出问题。eMMC 焊在板上，可靠性更高。

它要应对：

```text
OTA 写一半断电
日志长期写入磨损
数据库正在落盘
高温运行
文件系统损坏
```

建议 32GB，不要刚好够用。A/B、备份、日志、recovery 都会吃空间。

### 6.2 SPI NOR Flash

SPI NOR 是一小块更可靠的启动存储，通常放 bootloader，比如 U-Boot。

它的作用是：就算 eMMC 系统坏了，板子还有一个最底层的启动入口。

适合放：

```text
U-Boot
启动参数
当前 slot 状态
recovery 入口逻辑
```

不要指望 SPI NOR 放完整业务系统，空间太小。它更像门卫和救援开关。

### 6.3 Watchdog / Supervisor

Watchdog 是硬件看门狗。软件必须定期喂狗，如果新版本卡死、系统死循环、内核挂住，watchdog 就拉复位。

它应对：

```text
新版本启动后卡死
服务启动但不响应
系统进入死循环
远程 SSH 进不去
无人值守现场
```

工业 OTA 里它非常关键。没有 watchdog，软件挂死后就只能人工断电。

### 6.4 Brown-out Reset 电压监控

这是电压监控芯片。电压低到不安全时，它直接让 CPU 复位。

低电压时 CPU 可能还在跑，但 eMMC 写入已经不可靠。这种半死不活的状态最容易写坏文件系统。

它应对：

```text
24V 电源抖动
电机启动导致电压下陷
接触器动作造成瞬态压降
适配器老化
供电线太长压降
```

### 6.5 掉电检测

掉电检测不是复位，而是提前告诉 CPU：电源快没了。

它给软件一点时间做动作：

```text
停止 OTA 写入
停止数据库事务
flush 日志
把状态写成 interrupted
进入安全停机
```

工业现场最怕刚切 slot、刚写数据库、刚更新 boot 状态时断电。掉电检测就是为了争取最后几百毫秒到几秒。

### 6.6 保持电容 / 小 UPS 接口

掉电检测告诉系统快没电，但还需要能量撑住最后动作。保持电容或小 UPS 就是这段时间的能量来源。

它不一定要撑几分钟，很多时候撑 1-5 秒就很有价值。

它应对：

```text
电源被误拔
空开跳闸
现场电源闪断
工人切换电源
OTA 正在提交版本
```

### 6.7 Recovery 按键 / 拨码开关

这是人工强制救援入口。

比如现场人员按住 Recovery 上电，设备不进业务系统，而是进恢复模式：

```text
不控制反应釜
开放恢复网页
允许 U 盘安装包
允许以太网重新刷包
允许切回 factory 版本
```

这个按钮很朴素，但现场价值巨大。屏幕黑了、系统坏了、网络断了，它还能给维护人员一条路。

### 6.8 UART 调试口

UART 是最底层的串口调试口，用来看 bootloader 和 Linux 启动日志。

它应对：

```text
设备完全无法启动
以太网没起来
HMI 黑屏
SSH 进不去
bootloader 找不到系统
```

建议预留 3.3V TTL 4pin：GND、TX、RX、3V3，丝印标识清楚。量产时可以不焊针，但焊盘要有。

### 6.9 以太网

工业 OTA 主通道建议用以太网，不要把 Wi-Fi 当唯一更新通道。

它应对：

```text
远程 OTA 下载
局域网维护
日志上传
失败后 recovery 网页
弱无线环境
强电磁干扰
```

以太网口要有 TVS、共模电感、隔离变压器。

### 6.10 USB Host

USB Host 用于 U 盘离线恢复。

它应对：

```text
设备没网络
OTA 包下载失败
客户现场不让接外网
远程平台不可用
需要人工带包恢复
```

Recovery 模式里可以支持插入 U 盘，识别签名安装包，一键恢复。

### 6.11 RTC

RTC 是实时时钟，带电池或超级电容后，断网断电后还能知道时间。

它应对：

```text
设备离线运行
日志时间不能乱
审计链需要准确时间
OTA 失败要记录发生时间
证书校验依赖时间
```

工业系统里时间错乱会让排障非常痛苦。

### 6.12 FRAM / EEPROM

这是小容量非易失存储。FRAM 比 EEPROM 更适合频繁写。

它可以保存：

```text
当前 slot 是 A 还是 B
新版本是否已提交
启动失败次数
OTA 状态
最后失败原因
```

为什么不都写 eMMC：OTA 状态可能频繁小写入，FRAM 更抗写、更适合这种用途。

### 6.13 状态 LED

LED 不是装饰，是现场语言。

建议至少：

```text
RUN：系统正常
UPDATE：正在更新
FAIL：启动/健康检查失败
RECOVERY：恢复模式
```

很多工业现场没有显示器，也没人接串口。LED 能让维护人员第一眼知道设备在干什么。

### 6.14 JTAG/SWD / Boot Mode 测试点

这是量产和深度救援用的。

它应对：

```text
bootloader 烧坏
SPI NOR 内容异常
SoC 启动模式配错
板子批量生产测试
极端救砖
```

不一定给客户用，但研发和售后要有路。

### 6.15 工业保护电路

这些不是 OTA 专用，但会直接影响 OTA 可靠性：

```text
电源 TVS
反接保护
保险丝 / 电子保险
浪涌保护
ESD 保护
RS485/CAN 隔离
以太网隔离
USB ESD
良好接地
存储电源去耦
```

工业现场有电机、接触器、变频器、长线缆、静电、浪涌。软件更新最怕这些干扰刚好撞上写存储。

## 7. 工业异常场景与硬件兜底

| 场景 | 需要的硬件支持 |
|---|---|
| OTA 写一半断电 | 掉电检测、保持电容、小 UPS、可靠 eMMC |
| 新版本启动卡死 | Watchdog / supervisor |
| A/B 两个槽都无法启动 | SPI NOR、recovery、UART、USB Host、以太网 |
| 现场没有网络 | USB Host 离线恢复 |
| 现场无人值守 | Watchdog、自动回滚、状态 LED |
| 电压波动导致异常写入 | Brown-out reset、掉电检测、存储电源去耦 |
| 文件系统损坏 | recovery、factory package、备份恢复 |
| HMI 黑屏 | UART、LED、recovery 网页 |
| SSH 进不去 | UART、Recovery 按键 |
| 证书时间校验失败 | RTC |
| OTA 状态反复写入 | FRAM / EEPROM |
| 强电磁干扰 | 接口隔离、TVS、ESD、良好接地 |

## 8. 安全原则

Fallback/recovery 不应该继续正常控制反应釜。

Fallback/recovery 的职责是：

```text
停止自动控制
保持安全输出
恢复软件
导出日志
重新安装稳定版本
```

真正的物理急停、继电器、安全 PLC，仍然要独立于上位机软件。A/B OTA 只能保证软件能回来，不能替代硬件安全链。

## 9. 当前软件实现补充

当前项目已经按应用级 A/B 做了软件兜底：

```text
更新不覆盖当前可用 slot
候选包先进入 inactive slot
新版本健康检查通过才写 committed
失败自动切回 previous slot
recovery/fallback 只负责救援，不负责继续生产控制
```

同时，OTA 脚本会在这些关键动作后执行 `sync`，减少提交阶段断电后的状态歧义：

```text
/var/lib/reactor-edge/ota/state.json 状态更新
候选 staging 内容写入
inactive slot 正式替换
systemd unit 替换
root OTA 工具替换
/opt/reactor-edge/current 和 previous 链接切换
兼容链接 bin/frontend/static/kiosk/backup.sh/health-check.sh 切换
```

软件侧还启用了开机 OTA 检查：如果设备断电发生在切换 `current` 之前，下一次启动会记录 `interrupted_before_switch`，继续运行原 current slot；如果断电时 OTA 状态停在 `switching`、`health_checking` 或 `rolling_back`，下一次启动 backend 前会先恢复 `previous` slot，并把状态记录为 `rolled_back_on_boot`。只有 OTA 脚本正在主动启动候选版本做健康检查时，才会通过 `/run/reactor-edge/` 下的临时标记允许新 slot 继续接受测试；这个标记会记录 OTA 脚本 PID 和进程启动身份，boot-check 只有确认该进程仍存在时才放行，脚本被 kill、SSH 断开或 marker 残留时会删除 marker 并按中断 OTA 回滚；这个标记重启后也会消失。如果状态已经是 `failed`，设备会保持 backend 停止并进入维护态，不能继续生产控制。更新或手动回滚进入 `failed` 的当下，脚本也会清除健康检查临时放行标记并停止 backend/kiosk，避免失败候选版本继续控制现场。

第三方集成通道也按同样的工业兜底原则处理：AINAS/MQTT 的 `set_targets` 先检查现场联锁，审计成功后、提交运行态前还要再次检查 emergency stop、manual lock、控制故障、下游状态和新鲜传感器样本。若审计后现场状态突变，任务会被拒绝且不提交新目标；若目标已经提交但 integration task 回执落库失败，系统会锁存 `last_control_error` 并关闭自动控制，等待维护清除故障，避免外部系统以为任务成功而现场缺少可追溯回执。

这只能降低风险，不能替代硬件掉电检测、保持电容、小 UPS、brown-out reset 和可靠 eMMC。硬件侧仍要保证在存储写入窗口内尽量不断电，或至少给 CPU 留出完成落盘的时间。

## 10. 一句话总结

```text
eMMC 负责装两套系统，
SPI NOR 负责最底层启动，
watchdog 负责卡死重启，
掉电/电压监控负责防写坏，
recovery/UART/USB/以太网负责救援，
FRAM/LED 负责记录和现场判断。
```

A/B OTA 不需要两套主控，但必须有可靠存储、可恢复启动链、断电保护、watchdog 和人工救援入口。
