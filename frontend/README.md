# ReactorOS Frontend Workspace

This folder is the PRD-aligned Vue HMI migration workspace.

The production board still serves `static/index.html`. Keep that file as the
deployable single-file artifact until this Vue build reaches feature parity.
New frontend work should be developed here with Vue 3, Vite, TypeScript,
Element Plus, ECharts, Pinia, and Vue Router, then built back into a single HTML
file before replacing the production artifact.

## Commands

```bash
npm run frontend:dev
npm run frontend:build
```

The build output goes to `frontend/dist/index.html` and is configured as a
single-file bundle. Do not point the board service at `frontend/dist` until the
new HMI has parity with `static/index.html`.

## Current Migration Slice

- `src/App.vue` provides the industrial HMI shell, navigation, health status,
  and local role login shortcuts.
- `src/router.ts` maps the PRD seven pages: realtime monitor, process control,
  AI decision, history data, audit log, Modbus debug, and system settings.
- `src/stores/plant.ts` centralizes backend access through Pinia. It reads
  `/health`, `/api/config/summary`, `/api/audit/logs`,
  `/api/modbus/registers`, and `/api/recommendations/latest`; it also persists
  the zh/en UI language in local storage.
- `src/views/MonitorView.vue` uses ECharts for the first migrated live trend
  panel. Live samples are loaded on demand so a fresh dev environment without
  pipeline data still opens cleanly.
- All seven PRD route views now render their primary visible UI blocks through
  the shared language state. Chromium verification checks both zh and en text
  for `/#/monitor`, `/#/control`, `/#/ai`, `/#/history`, `/#/audit`,
  `/#/modbus`, and `/#/settings`.
- Visual smoke evidence is archived at
  `output/playwright/prd-vue-stack-monitor-20260606.png`; current i18n evidence
  is archived as `output/playwright/vue-i18n-verification.json` and
  `output/playwright/vue-i18n-*.png`.

## Migration Rules

- Keep API calls strict. Sensor values must come from the backend pipeline.
- Use ECharts for industrial trends and status visualization.
- Split by industrial screen modules first: status bar, device tree, monitor
  charts, process control, AI control, audit, Modbus, and settings.
- Preserve the current browser budget: no required external first-screen
  requests, short live history windows, and touch-sized controls.
