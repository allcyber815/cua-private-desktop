# Private desktop test fixtures

These sources are owned by the WebGPT CUA private-desktop bundle in the MCP control plane. They replace the historical dependency on the large `Dev\WebGPT\experiments\computer-use-background-architecture-20260922` and `agent-computer-use-canary` trees.

Runtime artifacts are materialized on demand under the repository's ignored `.deps\private-desktop-fixtures` directory by:

```powershell
pwsh -File scripts\prepare-private-desktop-fixtures.ps1 -Force
```

Pinned external fixture dependencies:

- Microsoft.Web.WebView2 1.0.3179.45
- Microsoft.WindowsAppSDK 1.8.260317003
- Microsoft.Windows.SDK.BuildTools 10.0.26100.7175
- PySide6-Essentials 6.11.2
- Electron 44.4.2

The prepared runtime is a disposable cache, not source of truth. `bin/` and `obj/` output must never be retained under `test-fixtures/`; the preparation script redirects build intermediates and outputs into `.deps`. Browser/Electron profiles used by tests must be per-run temporary directories rather than persistent fixture state.
