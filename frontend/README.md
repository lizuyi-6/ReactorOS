# ReactorOS Frontend Workspace

This folder is the development workspace for the future componentized HMI.

The production board still serves `static/index.html`. Keep that file as the
deployable single-file artifact. New frontend work should be developed here with
Vite + TypeScript, then built back into a single HTML file before replacing the
production artifact.

## Commands

```bash
npm run frontend:dev
npm run frontend:build
```

The build output goes to `frontend/dist/index.html` and is configured as a
single-file bundle. Do not point the board service at `frontend/dist` until the
new HMI has parity with `static/index.html`.

## Migration Rules

- Keep API calls strict. Sensor values must come from the backend pipeline.
- Keep Canvas charting lightweight; do not add Chart.js or large visualization
  dependencies for the board HMI.
- Split by industrial screen modules first: status bar, device tree, monitor
  charts, process control, AI control, alarms, and settings.
- Preserve the current browser budget: no required external first-screen
  requests, short live history windows, and touch-sized controls.
