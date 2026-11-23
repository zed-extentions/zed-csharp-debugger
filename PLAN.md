# PLAN — `csharp-debugger`: a C#/.NET debug adapter extension for Zed

> Reimplementing an open-source VSCode debug extension as a Zed DAP extension.
> Research date: 2026-10-29 · zed_extension_api 0.7.0 · Zed 1.22.0 (build target corrected to wasm32-wasip2, 2026-10-30)

## 1. Research

### 1.1 Debugger language coverage in the Zed store/official docs (2026-10-29)

| Language | Adapter | Provided by | Status |
|---|---|---|---|
| C / C++ | CodeLLDB, gdb | Zed built-in (`crates/dap_adapters/`) | covered |
| Rust | CodeLLDB (+ probe-rs, BugStalker extensions) | built-in + store | covered |
| Go | delve | Zed built-in | covered |
| JavaScript / TypeScript | vscode-js-debug | Zed built-in | covered |
| Python | debugpy | Zed built-in | covered |
| PHP | php-debug-adapter | built-in family (`zed-extensions/php`) | covered |
| Java | java-debug | extension (`zed-extensions/java`) | covered |
| Ruby | rdbg | extension (`zed-extensions/ruby`) | covered |
| Swift | lldb-dap | extension (`zed-extensions/swift`) | covered |
| Lua | EmmyLua | store extension | covered |
| AutoHotkey | AHK DAP | store extension | covered |
| Embedded Rust | probe-rs | store extension | covered |
| Unity | Unity Debugger | store extension | covered |
| **C# / .NET** | **none** | — | **gap** |
| Kotlin, Elixir, Haskell, GDScript, … | incomplete code in some repos | not in docs/store | de-facto gaps |

Sources:
- Store `?filter=debug-adapters` listing: probe-rs, EmmyLua, AutoHotkey, BugStalker, Unity Debugger
- Store search `netcoredbg` → "No extensions found"
- https://zed.dev/docs/debugger — built-ins: C, C++, Go, JavaScript, PHP, Python, Rust, TypeScript / via extensions: Java, Ruby, Swift (no C#)
- https://zed.dev/languages/csharp — "Native C# debugging isn't available yet"
- Third-party `qwadrox/zed-netcoredbg` exists on GitHub but is **not in the store** (README covers manual dev-extension install only)

### 1.2 Candidate evaluation

| Candidate | License | Zed coverage | Verdict |
|---|---|---|---|
| microsoft/vscode-js-debug (JS/TS) | MIT | Zed built-in | rejected (covered) |
| microsoft/debugpy (Python) | MIT | Zed built-in | rejected (covered) |
| eclipse/java-debug (Java) | MIT | Java extension | rejected (covered) |
| ruby/vscode-rdbg (Ruby) | MIT | Ruby extension | rejected (covered) |
| **Samsung/netcoredbg (C#/.NET)** | **MIT** | **store gap + official "unsupported"** | **selected** |

### 1.3 Selection: C# / .NET — netcoredbg

Rationale:
1. **Demand**: C#/.NET is a top-tier language ecosystem and Zed's own docs declare the
   debugging gap explicitly.
2. **Confirmed gap**: neither the store's debug-adapters filter nor search surfaces a
   C#/netcoredbg extension. Microsoft's official vsdbg is VSCode-only and not
   redistributable (proprietary), making netcoredbg the de-facto only open-source route —
   and it is not in the store.
3. **License**: Samsung/netcoredbg is **MIT** — reimplementing the DAP configuration
   dialect (`--interpreter=vscode`, launch/attach arguments) has no legal constraints. This
   extension contains no netcoredbg code and does not distribute the binary.
4. **v1 fit**: netcoredbg is a user-installed CLI (`netcoredbg --interpreter=vscode` = stdio
   DAP), matching the v1 model exactly. Only the launch.json → adapter wiring that the
   VSCode C# extension performed needs reimplementation.

### 1.4 netcoredbg DAP contract (verified in `src/protocols/vscodeprotocol.cpp`)

- Execution: `netcoredbg --interpreter=vscode` (stdio DAP server)
- **launch** arguments: `program` (required; a `.dll` runs via `dotnet <dll>`, otherwise
  treated as an executable), `args` (string[]), `cwd`, `env` (map), `stopAtEntry` (default
  false), `justMyCode` (default true), `enableStepFiltering` (default true)
- **attach** arguments: `processId` (required, integer or numeric string)

### 1.5 Extension API (verified against the crate source + WIT, 0.7.0)

```rust
fn get_dap_binary(&mut self, adapter_name: String, config: DebugTaskDefinition,
    user_provided_debug_adapter_path: Option<String>, worktree: &Worktree)
    -> Result<DebugAdapterBinary, String>;
fn dap_request_kind(&mut self, adapter_name: String, config: serde_json::Value)
    -> Result<StartDebuggingRequestArgumentsRequest, String>;
fn dap_config_to_scenario(&mut self, config: DebugConfig) -> Result<DebugScenario, String>;
```

`DebugAdapterBinary { command: Option<String>, arguments: Vec<String>, envs: Vec<(String,String)>, cwd: Option<String>, connection: Option<TcpArguments>, request_args: StartDebuggingRequestArguments }`
- `StartDebuggingRequestArguments { configuration: String (JSON), request: Launch|Attach }`
- stdio adapters use `connection: None`
- `Worktree::{which(&str) -> Option<String>, root_path() -> String, shell_env()}`
- The path a user sets in `dap.netcoredbg.binary` arrives as `user_provided_debug_adapter_path`
  (verified in zed's `extension_dap_adapter.rs`; args/env forwarding is a Zed-side TODO)

extension.toml manifest: `[debug_adapters.<name>]` + `schema_path` (default location
`debug_adapter_schemas/<name>.json`), `schema_version = 1`.

## 2. Architecture

```
.zed/debug.json (user) ──▶ Zed debugger UI
     │ DebugTaskDefinition { label, adapter: "netcoredbg", config: JSON }
     ▼
┌─ extension (wasm32-wasip2) ────────────────────────────┐
│ lib.rs        Extension trait impl (WASM API touchpoint)│
│ resolve.rs    [pure] binary lookup / args / DOTNET_ROOT  │
│ config/       [pure] launch/attach normalization         │
└──────────────────────────────────────────────────────────┘
     │ DebugAdapterBinary { command: netcoredbg path,
     │   arguments: ["--interpreter=vscode"], connection: None, ... }
     ▼
netcoredbg process (stdio DAP) ──▶ .NET app debugging
```

- **Binary acquisition (v1)**: ① user-set `dap.netcoredbg.binary` → ②
  `worktree.which("netcoredbg"/".exe")` PATH probe → ③ instructive error. Automatic
  download is a phase-2 task (§6).
- **DOTNET_ROOT hint**: at launch, if the user env lacks `DOTNET_ROOT` and `dotnet` is on
  PATH, its parent directory is injected (mitigates the common Windows runtime-discovery
  failure).
- **Config normalization**: user JSON parsed as `serde_json::Value`, required-field
  validation (program/processId), defaults (cwd ← worktree root), unknown fields preserved
  and forwarded verbatim (maximal vscode-csharp launch.json compatibility).
- **Module separation**: everything touching `zed_extension_api` types/WASM imports stays
  in `lib.rs`; the rest is pure logic (serde_json Value + primitives) verified by host
  `cargo test`.

## 3. Extension schema

- Extension id: `csharp-debugger` (store-collision-free)
- Adapter name: `netcoredbg` (matches the user's debug.json `adapter` field and `dap.netcoredbg.*` settings keys)
- `debug_adapter_schemas/netcoredbg.json`: a launch/attach oneOf JSON Schema used by Zed to validate/complete debug.json

## 4. Test strategy

1. **Pure unit tests (`cargo test`, host)** covering resolve/config:
   - request-kind detection (launch/attach/missing/invalid)
   - launch normalization: program required, cwd default, env merge/DOTNET_ROOT injection, unknown-field preservation
   - attach normalization: processId number/string coercion, missing-value error
   - binary resolution priority (user path > which > instructive error)
   - platform-specific binary candidates and adapter arguments (`--interpreter=vscode`)
2. **Build verification**: `cargo build --target wasm32-wasip2 --release`
3. **E2E**: not verifiable headlessly → manual procedure documented in the README (sample .NET console app, break/step/variables, attach)

## 5. Deliverables

| File | Purpose |
|---|---|
| `extension.toml` | id + debug_adapters registration |
| `Cargo.toml` | cdylib, zed_extension_api 0.7, serde_json |
| `src/lib.rs` | Extension trait implementation |
| `src/resolve.rs`, `src/config/` | pure logic + unit tests |
| `debug_adapter_schemas/netcoredbg.json` | JSON schema for debug.json |
| `README.md` | usage / manual verification / licensing |
| `LICENSE` | MIT (this extension's code) |

## 6. Phase 2 (follow-ups)

- Automatic netcoredbg download (GitHub Releases + `download_file` / `latest_github_release`)
- `--engine_logging` diagnostics, richer `sourceFileMap`/`targetArchitecture` schema
- Zed locators (`[debug_locators]`) turning `dotnet build` tasks into debug scenarios
- User-custom adapter args/env once Zed forwards `dap.*.args/env`

---

## 7. Two-Phase Evolution Roadmap (Zero VSCode Dependency + Zed-Native Evolution)

- **Phase 1 (Completed Baseline)**: Zero dependency on proprietary `vsdbg` or VSCode extensions; pure Rust DAP configuration normalizer (`config/`), `DOTNET_ROOT` discovery (`resolve.rs`), and Samsung `netcoredbg` GitHub Release auto-provisioning.
- **Phase 2 (Zed-Native Evolution)**:
  1. **P2-1 (`.csproj` / `.sln` Auto-Discovery Debug Locators)**: Turn `dotnet build` / `dotnet test` Zed tasks into zero-config debug scenarios without requiring manual `.zed/debug.json` authoring.
  2. **P2-2 (Multi-Target Framework & Blazor/ASP.NET Core Profiles)**: Automatic `targetFramework` (`net8.0`/`net9.0`) DLL path inference and `launchSettings.json` environment variable import.
