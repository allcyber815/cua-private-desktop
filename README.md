# CUA Private Desktop Overlay

This repository is the canonical source for the WebGPT-maintained Windows CUA Driver private-desktop overlay and its public Windows build/verification workflow.

## Upstream

- Based on: https://github.com/trycua/cua
- Upstream component: CUA Driver
- Pinned upstream source: see `cua-private-desktop/upstreams.toml`
- Upstream license: MIT
- Upstream copyright: Copyright (c) 2025 Cua AI, Inc.

This repository maintains an overlay separately from upstream CUA. Its presence here does **not** imply that upstream CUA has adopted or endorsed these changes.

## Layout

- `cua-private-desktop/overlay/` — tracked patch plus overlay files
- `cua-private-desktop/scripts/` — sparse materialization, capture, low-disk verification, conformance, and remote-build scripts
- `cua-private-desktop/test-fixtures/` — private-desktop conformance fixtures
- `cua-private-desktop/upstreams.toml` — exact upstream version/commit pin
- `.github/workflows/cua-private-desktop-remote-build.yml` — public Windows deep-build lane

## Build model

Ordinary source work uses the low-disk sparse materialization flow and does not require a full upstream clone or a preserved Cargo target tree.

Deep Windows compilation/test/package verification runs on GitHub-hosted `windows-latest` runners through the checked-in workflow. The workflow intentionally uses read-only repository permissions and requires no repository secrets.

## Public-repository security model

Treat workflow logs and uploaded build artifacts as public data. Do not commit runtime profiles, machine state, credentials, local paths, tunnel state, crash dumps, caches, or generated deployment material.

## License

MIT. See `LICENSE`.
