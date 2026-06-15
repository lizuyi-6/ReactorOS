# Repository Map

This workspace is a mixed Rust + Vue + evidence repo.

- `src/`: Rust daemon, API, storage, control, safety, and integration code
- `frontend/src/`: Vue HMI shell, store, router, and view components
- `frontend/src/styles/`: layered HMI CSS modules; route-only layout rules live under `styles/routes/`
- `scripts/`: packaging, smoke checks, replay helpers, and verification gates
- `tests/`: Rust integration and unit tests
- `docs/`: operator docs, acceptance docs, and evidence indexes
- `output/`: generated evidence and local scratch artifacts
- `outputs/`: archived training and delivery bundles

Local scratch directories that should stay out of git:

- `output/dev/`
- `output/playwright/`
- `output/compliance-audit-report-*.md`
