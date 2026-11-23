# Requirements Specification — zed-csharp-debugger (C#/.NET Debug Adapter)

- Version: v1.1 · Status: **requirements approved**
- Written: 2026-10-30 · Adapter: integration with [Samsung/netcoredbg](https://github.com/Samsung/netcoredbg) (MIT)
- Priorities: MUST (required in v1) / SHOULD (recommended) / LATER (backlog)

## 1. Goal

Enable launch/attach debugging of C#/.NET applications from Zed's debug UI. Fills the C#
debugging gap confirmed in the Zed store and official docs (netcoredbg absent). The
extension ships no netcoredbg binary in the repo; the binary is user-installed
(PATH or `dap.netcoredbg.binary`) or downloaded automatically from GitHub Releases at
first use (FR-16).

## 2. Functional requirements

### Adapter execution

| ID | Requirement | Priority | Verification |
|---|---|---|---|
| FR-1 | `get_dap_binary` returns a `DebugAdapterBinary` that runs the `netcoredbg` adapter with `--interpreter=vscode` (stdio DAP) | MUST | G2·G5 |
| FR-2 | Binary resolution priority: ① user setting path (`dap.netcoredbg.binary`) ② `worktree.which()` over the platform candidates (Windows: `netcoredbg`/`netcoredbg.exe`, Unix: `netcoredbg`) ③ automatic download (FR-16) ④ instructive error with the Releases URL, build docs and a settings example | MUST | G2 |
| FR-3 | A whitespace-only user path is treated as absent (PATH fallback preserved) | MUST | G2 |
| FR-4 | The adapter process cwd is the worktree root | MUST | G5 |
| FR-5 | Stdio DAP (`connection: None`) by default; when `tcp_connection` is supplied in `DebugTaskDefinition`, resolve it with `resolve_tcp_template` and return `command: None`, `arguments: []`, `connection: Some(...)` (FR-25) | MUST | G2·G5 |
| FR-16 | Automatic download: when no user path or PATH hit exists, the latest Samsung/netcoredbg release is resolved via `latest_github_release` (or `github_release_by_tag_name` when the `VERSION_PIN` constant is set), the asset for `current_platform()` is selected (pure matrix in `download.rs`; macOS x86_64 falls back to tag `3.1.3-1062`, the newest release with `osx-amd64`), downloaded with `download_file` whose `path` is the version directory `netcoredbg/<version>/` (archives are extracted into it; binary at `netcoredbg/<version>/netcoredbg/netcoredbg[.exe]`) and made executable; the resolved path is memoized for the Zed session; an extracted version is reused without downloading (or re-downloaded with `ZED_NETCOREDBG_FORCE_UPDATE` set); older version directories are removed after a successful download; lookup/download failures fall back to the newest cached version, and to the instructive error when nothing is cached | SHOULD | G2·G5 |

### launch configuration

| ID | Requirement | Priority | Verification |
|---|---|---|---|
| FR-6 | Identifies `request: "launch"` and requires `program` (non-string/blank rejected, trimmed and normalized) | MUST | G2 |
| FR-7 | `cwd` defaults to the worktree root when absent; explicit values are preserved | MUST | G2 |
| FR-8 | `args` must be an array of strings and `env` an object of string→string (violations produce clear errors) | MUST | G2 |
| FR-9 | `DOTNET_ROOT` hint: when absent from the user's env and a `dotnet` executable is found, its parent directory is injected into both the launch env and the adapter process env (case-insensitive duplicate guard) | MUST | G2·G5 |
| FR-10 | Unknown fields (`sourceFileMap`, …) are preserved and forwarded to netcoredbg verbatim (forward compatibility) | MUST | G2 |

### attach configuration

| ID | Requirement | Priority | Verification |
|---|---|---|---|
| FR-11 | Identifies `request: "attach"` and requires `processId` (integer or numeric string, normalized to an integer) | MUST | G2 |
| FR-12 | Extra fields in attach configurations are preserved | MUST | G2 |

### Common

| ID | Requirement | Priority | Verification |
|---|---|---|---|
| FR-13 | Missing/invalid `request` fields, JSON parse failures and non-object configs produce actionable error messages | MUST | G2 |
| FR-14 | `debug_adapter_schemas/netcoredbg.json` — validates and completes `.zed/debug.json` editing for launch/attach, including `logging`, `launchSettingsProfile`, `launchSettingsFilePath`, `envFile`, `console` (`["internalConsole"]`), and `pipeTransport`; `justMyCode` and `enableStepFiltering` apply to launch only (removed from attach, where netcoredbg reads only `processId`); `sourceFileMap` is accepted for vscode-csharp compatibility but documented as ignored by netcoredbg (no `targetArchitecture`: netcoredbg does not support it) | SHOULD | G5 |
| FR-15 | Requests for any adapter name other than ours error out (single-adapter principle) | MUST | G2 |
| FR-18 | Diagnostics: the adapter config `logging` object maps to netcoredbg adapter flags — `engine` (or vscode-csharp's `engineLogging`) → `--engineLogging` (engine/DAP activity reported as console output events in Zed's debug console), `enginePath` → `--engineLogging=<path>` (implies `engine`), `log` → `--log` (internal log in a temp file), `logPath` → `--log=<path>` (implies `log`; `"stdout"` rejected because it would corrupt the DAP stream); the object is stripped from the forwarded configuration; the other vscode-csharp `logging` keys (`exceptions`, `moduleLoad`, `programOutput`, `browserStdOut`, `elapsedTiming`, `threadExit`, `processExit`, `consoleUsageMessage`, `diagnosticsLog`) are ignored without error; other unknown keys and wrong types are rejected. Log paths resolve against the worktree root and their directory must exist (netcoredbg drops the log silently otherwise). Flag names verified against netcoredbg `src/main.cpp` (`--engineLogging`, camelCase — `--engine_logging` is an unknown option) | SHOULD | G2 |
| FR-19 | Zed session entry points: `dap_request_kind` returns launch/attach from the scenario's `request` field (delegating to the pure `config::request_kind`; errors when it cannot be determined, never a default), so plain `.zed/debug.json` scenarios start without a locator; `dap_config_to_scenario` converts the new-process modal's debugger-agnostic `DebugConfig` into our configuration — launch: `program`, `args`/`cwd`/`env` when present, `stop_on_entry` → `stopAtEntry`; attach: `processId` only when a PID is given (never `null`); no `build` step | MUST | G2·G5 |

### Debug locator (build-then-debug)

| ID | Requirement | Priority | Verification |
|---|---|---|---|
| FR-17 | A `dotnet build` / `dotnet test` locator (registered as `debug_locators.dotnet-build`, with a `process:exec` capability for `dotnet msbuild **`): phase 1 (`dap_locator_create_scenario`) offers a debug scenario for `dotnet build` and `dotnet test` tasks — `command` `dotnet`/`dotnet.exe` (optionally a path) with first argument `build` or `test`, or the single-string form `"command": "dotnet build ..."` / `"dotnet test ..."` — rewriting `dotnet test` to a `dotnet build` step (excluding test-only flags) so failing tests do not abort the pre-launch build; after Zed runs the build, phase 2 (`run_dap_locator`) asks MSBuild for the output via `dotnet msbuild <target> -getProperty:TargetPath;TargetFramework;TargetFrameworks;MSBuildProjectFullPath` (target: explicit project/directory argument resolved against the task `cwd`, or the `cwd` itself) with the task's `-c/--configuration`, `-f/--framework`, `-r/--runtime`, `--ucr`, `-o/--output`, `--artifacts-path` and `-p:/-property:/--property:` switches forwarded as global properties the way the `dotnet` CLI forwards them (and ignoring `dotnet test` switches / `--` runner args); the launch `program` is MSBuild's `TargetPath` and `cwd` the project directory; the converted request goes through `dap_config_to_scenario` (FR-19); explicit `program` values still take precedence (Zed merges the user config over the locator output); build failures, missing `.NET SDK 8+`, missing absolute task `cwd`, solution targets (`.sln/.slnx/.slnf` argument, or a directory containing only a solution — MSB1063: point the task at a project), several projects in the directory, `-a/--arch`/`--os` (use `-r`) and multi-target projects without `-f` surface as actionable errors without starting a session | SHOULD | G2·G5 |
| FR-20 | Process picker macro: `normalize_attach` preserves `${ZED_PICK_PID}` / `$ZED_PICK_PID` intact so Zed substitutes the interactively chosen PID | MUST | G2·G5 |
| FR-21 | `.vscode/launch.json` bridge: when a configuration lacks `request` or `program`/`processId`, read `.vscode/launch.json` (JSONC) from the worktree, select the matching or first `coreclr` configuration, translate `${workspaceFolder}`/`${workspaceRoot}` and `${command:pickProcess}` → `${ZED_PICK_PID}`, and merge missing fields into the launch/attach configuration | SHOULD | G2·G5 |
| FR-22 | `launchSettings.json` profiles: support `launchSettingsProfile` and `launchSettingsFilePath` (default `Properties/launchSettings.json` when `launchSettingsProfile` is given without `launchSettingsFilePath`) in launch configs; read via `Worktree::read_text_file`, strip JSONC comments/trailing commas, select the named profile or the first with `"commandName": "Project"`, merge `environmentVariables` → `env` (explicit `env` wins), `applicationUrl` → `env["ASPNETCORE_URLS"]` (if absent), `commandLineArgs` → `args` (if explicit `args` absent), `workingDirectory` → `cwd` (if absent/blank), ignore `launchBrowser`, error clearly on missing file/profile, and strip `launchSettings*` keys before forwarding | SHOULD | G2·G5 |
| FR-23 | `envFile` dotenv support: support a worktree-relative `envFile` string on launch configs (normalizing `${ZED_WORKTREE_ROOT}/`, `${workspaceFolder}/`, and `\`), parse `KEY=VALUE` lines (`#` comments, blank lines, optional `export `, matching single/double quotes), merge with precedence `env` > `launchSettings` > `envFile` > `DOTNET_ROOT` hint, reject non-string/invalid paths or unreadable files, and strip `envFile` before forwarding | SHOULD | G2·G5 |
| FR-24 | `console` option: accept `"internalConsole"` and strip it before forwarding; reject `"integratedTerminal"`, `"externalTerminal"`, or other values with an actionable error explaining netcoredbg lacks DAP `runInTerminal` (no stdin/TTY) and linking the Zed task + `"processId": "${ZED_PICK_PID}"` attach workaround | SHOULD | G2·G5 |
| FR-25 | Remote / container debugging: support `"pipeTransport": { "pipeProgram", "pipeArgs", "debuggerPath" }` (building `command = pipeProgram`, `arguments = pipeArgs + [debuggerPath, "--interpreter=vscode"] + logging_args`, skipping local binary lookup and `DOTNET_ROOT` injection, stripping `pipeTransport`) and TCP connect when `Definition.tcp_connection` is `Some(...)` (resolving via `resolve_tcp_template`, returning `command: None`, `arguments: []`, `connection: Some(...)`, skipping local `DOTNET_ROOT` injection); reject combining both | SHOULD | G2·G5 |
| FR-26 | Test debugging (`dotnet test`): extend the locator to build + locate Microsoft.Testing.Platform (MTP) test executables from `dotnet test` tasks while stripping/ignoring test-only flags, and document MTP launch (`--filter-method`) and `VSTEST_HOST_DEBUG=1 dotnet test` + `${ZED_PICK_PID}` attach recipes in `README.md` | SHOULD | G2·G5 |
| FR-27 | Capability limits & attach schema alignment: document netcoredbg vs `vscode-csharp`/`vsdbg` capability limits (exception filters `all`/`user-unhandled`, conditional/function breakpoints, unsupported logpoints/hit-counts/hot-reload/`sourceFileMap`) in `README.md` and remove no-op `justMyCode`/`enableStepFiltering` from the attach schema | SHOULD | G5 |
| FR-28 | Document Zed API/UI-blocked `vscode-csharp` features (user adapter `args`/`env`, `serverReadyAction`/`launchBrowser`, `preLaunchTask`/`postDebugTask`, `pickProcess`, `coreclr` importer mapping, `checkForDevCert`, default exception filters) with their workarounds in a `README.md` "Known Zed limitations & workarounds" section, linking issue #16 | SHOULD | G5 |

### Backlog (out of scope for v1)

| ID | Requirement | Priority |
|---|---|---|
| — | Upstream Zed API / UI blocked items tracked in [issue #16](https://github.com/zed-extentions/zed-csharp-debugger/issues/16) | LATER |

## 3. Non-functional requirements

| ID | Requirement |
|---|---|
| NFR-1 | `config/`, `resolve.rs`, `locator/` and `download.rs` stay independent of `zed_extension_api` host imports (`std`, `zed_extension_api::serde_json` and the vetted pure-Rust crates listed in `.rules` — currently `jsonc-parser` for JSONC parsing) — verified by host `cargo test`. Zed API touchpoints stay in `lib.rs` |
| NFR-2 | Build target `wasm32-wasip2` |
| NFR-3 | No netcoredbg binaries or sources are committed (licensing/size) — integration only; runtime download goes to the extension working directory, never into the repo |
| NFR-4 | Error messages guide installation (download/build/settings) |
| NFR-5 | MIT license; the netcoredbg/vscode-csharp relationship is documented in the README |
| NFR-6 | Pure logic handles both Windows path separators (`/`, `\`) |
| NFR-7 | Platform coverage documented in `README.md` (`linux-amd64`, `linux-arm64`, `osx-arm64`, `osx-amd64` `3.1.3-1062`, `win64`, plus `win-arm64` / Linux `musl` build-from-source guidance); unsupported-platform error links to `https://github.com/Samsung/netcoredbg#building-from-source` |

## 4. Constraints

- Zed does not yet forward `dap.*.args/env` user customizations (binary path only) — hence the compile-time `VERSION_PIN` constant instead of a settings-based version pin
- E2E debug sessions cannot be verified headlessly → G5 manual procedure (README)

## 5. Traceability

FR-1..27 implemented (G2 159/159, G3 build passed).
netcoredbg DAP contract source: `src/protocols/vscodeprotocol.cpp` (see PLAN.md §1.4).
MSBuild `-getProperty` output shape (`{"Properties": {...}}`), `TargetPath` for `-c/-r/-o/--artifacts-path`, MSB1063 on solutions and empty `TargetPath` for multi-target projects verified on .NET SDK 9.0.316; CLI switch → property forwarding per dotnet/sdk `GivenDotnetBuildInvocation.cs`.
