# Upper-Computer Acceptance Report

- commit: `f7ac7d17`
- base URL: `http://127.0.0.1:18300`
- Vue URL: `http://127.0.0.1:15173/`
- steps pass / fail / total: **20 / 0 / 20**
- final status: **OK**

## Steps

| Step | Status | Info |
|---|---|---|
| `verify-vue-release-assets` | ok | release package and systemd paths prefer Vue dist with legacy fallback |
| `verify-production-safety-guard` | ok | release package and systemd launch the isolated safety guard |
| `verify-production-backup-schedule` | ok | systemd timer schedules online SQLite backup snapshots |
| `verify-production-backup-script` | ok | backup script writes timestamped SQLite snapshots and latest link |
| `verify-backup-restore-drill` | ok | restored snapshot boots a fresh daemon with batch, product result, and audit chain intact |
| `verify-training-deliverables` | ok | training deck, PPTX package, image assets, UAT script, and preview manifest passed |
| `xingshu-ops-preflight` | ok | production secrets, TLS paths, and backup timer files checked |
| `vite-dev` | ok | vite dev on 15173 proxied to 18300 |
| `verify-load-and-rbac` | ok | RBAC matrix all-pass; see X:\tianhks\output\acceptance\logs\load-and-rbac.log |
| `verify-vue-parity` | ok | Vue 7 routes and bilingual checks passed |
| `verify-vue-history-xlsx` | ok | History CSV/XLSX downloads and bilingual buttons passed |
| `verify-vue-process-lifecycle` | ok | process lifecycle and bilingual checks passed |
| `verify-vue-mobile` | ok | phone and tablet viewport bilingual navigation checks passed |
| `verify-vue-browser-matrix` | ok | passed browsers: chromium, chrome, msedge, firefox, webkit; skipped: ; page checks: 70; console errors: 0 |
| `probe-cli-ops` | ok | real SQLite backup/restore/wipe/key generate/key rekey |
| `verify-ainas-mqtt` | ok | AINAS API and integration config summary passed |
| `verify-mosquitto-broker` | ok | real broker status/task/receipt round-trip passed |
| `mock-entrypoints-parse` | ok | AINAS/STM32 mock entrypoints parse |
| `ainas-mock-health` | ok | AINAS mock /health returned 200 on 127.0.0.1:5599 |
| `stm32-modbus-mock-fc03` | ok | STM32 mock answered Modbus TCP FC03 on 127.0.0.1:15502 |

## Report Files

- JSON: `output/acceptance/acceptance-report.json`
- Markdown: `output/acceptance/acceptance-report.md`
- logs: `output/acceptance/logs/`
