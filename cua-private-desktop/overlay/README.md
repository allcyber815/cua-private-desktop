# CUA private-desktop overlay

This directory captures the WebGPT private-desktop delta rebased onto the exact CUA 0.34.0 commit pinned in the bundle's `upstreams.toml`. The semantic rebase preserves CUA 0.34.0's element-token-only action addressing, snapshot replacement/invalidation behavior, walk budgets, screenshot ownership, timeout, and browser-installation safety contracts while retaining the WebGPT private-desktop semantic/value actions, background input delivery, attached UIA, trusted PrintWindow capture, interference receipts, and session-aware verification. GitHub Actions Windows is the authoritative deep-build lane; when that lane is unavailable, the same pinned build pipeline can be run explicitly on the local Windows host with `-AllowLocalBuild`.

- `tracked.patch` contains modifications to files already tracked by upstream CUA, including the additive ABI 1.2 daemon-connect seam retained by the promoted CUA private-desktop overlay.
- `files/` contains new text/source files introduced by the private-desktop implementation.
- experimental WGC probe programs and generated binary fixtures are intentionally excluded from the promoted overlay.

## Low-disk development contract

Ordinary development must not require a full CUA clone, a preserved Cargo target tree, or the multi-provider fixture runtime.

- `scripts\materialize-cua.ps1` fetches one exact commit with `--depth=1 --filter=blob:none` and sparsely hydrates `libs/cua-driver/rust`, `libs/cua`, `libs/fleet`, and `libs/images`: the minimal source/workspace closure required by CUA 0.34.0 metadata/format validation and `cua-telemetry` compile-time assets.
- Routine rebase checks are source-only: patch preflight/application, `cargo +stable metadata --no-deps --locked`, and `cargo +stable fmt --all -- --check`.
- Do not invoke bare `cargo` from this checkout when the pinned upstream toolchain is not already installed; an explicit already-installed compatible toolchain avoids an unnecessary Rust toolchain download.
- Cargo check/build/test output normally comes from the GitHub Actions Windows release-candidate lane. `scripts\build-cua-remote.ps1` still refuses local deep builds by default; `-AllowLocalBuild` is an explicit fallback for an unavailable remote lane and runs the same materialization, regression, release-build, signed-helper, and receipt checks with the exact pinned Rust 1.97.1 toolchain. Keep fallback target/output trees disposable.
- Provider-heavy WinUI/WebView2/XAML Island/Qt/Electron/Chromium fixtures remain explicit opt-in conformance inputs. They are allowed during development when they materially reduce integration risk; build them only for the verification that needs them and remove them afterward.

## Test fixture contract

The default private-desktop semantic test does not require a preserved experiment tree or committed binary fixture. It compiles `tests/fixtures/WpfStableFixture.cs` into `%TEMP%` at test time with the Windows .NET Framework compiler discovered under `%WINDIR%\\Microsoft.NET`.

Provider-specific historical conformance tests are explicit opt-in tests (`#[ignore]`). The canonical replay path is repository-owned source -> disposable prepared runtime -> explicit environment contract:

```powershell
pwsh -File scripts\prepare-private-desktop-fixtures.ps1 -Force
pwsh -File scripts\run-private-desktop-conformance.ps1 -ForceMaterialize
```

The preparation script builds only into the ignored `.deps\private-desktop-fixtures` runtime, including disposable NuGet and npm caches; the conformance runner keeps Cargo home and target output under `.deps` as well. Use `-ForceMaterialize` when replaying after an overlay source change so the disposable pinned-CUA checkout is rebuilt from the current overlay instead of reusing a stale `.deps\cua-private-desktop` tree. `test-fixtures\private-desktop` remains source-only; generated `bin/` and `obj/` content is not canonical and is ignored defensively.

The lower-level environment contract used by the runner is:

- `WEBGPT_CUA_WINFORMS_INVOKE_FIXTURE` — WinForms Invoke fixture executable.
- `WEBGPT_CUA_WINFORMS_NATIVE_FIXTURE` — WinForms Toggle/Select fixture executable.
- `WEBGPT_CUA_CONFORMANCE_ROOT` — root containing the WinUI, WebView2, XAML Island, Qt, Electron-app, Chromium HTML, Tk, and helper assets used by the historical provider matrix.
- `WEBGPT_CUA_PYTHONW_EXE` — Python GUI interpreter for Qt/Tk cells.
- `WEBGPT_CUA_ELECTRON_EXE` — Electron executable for the Electron cell.
- `WEBGPT_CUA_CHROME_EXE` — Chrome/Chromium executable for the Chromium cell.

Electron and Chromium test profiles are created under `%TEMP%` and removed after a successful test. External provider runtimes and generated binaries remain non-canonical, regenerable conformance inputs rather than repository or production-runtime dependencies.

The overlay is maintained CUA source owned by this repository, not a claim that CUA upstream has adopted these changes. Materialization must start from the pinned commit and fail if the patch no longer applies cleanly.
