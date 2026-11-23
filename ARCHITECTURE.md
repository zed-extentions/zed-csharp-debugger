# Architecture Specification — zed-csharp-debugger (`csharp-debugger`)

> **Upstream References**:
> - Debugger Engine: [Samsung/netcoredbg](https://github.com/Samsung/netcoredbg) (MIT) — `src/main.cpp`, `src/protocols/vscodeprotocol.cpp`
> - Configuration Dialect: [dotnet/vscode-csharp](https://github.com/dotnet/vscode-csharp) (MIT) — `coreclr` launch/attach schema, `launchSettings.json` profile merger, `pipeTransport`, & `logging` options
> - CLI-to-MSBuild Mapping: [dotnet/sdk](https://github.com/dotnet/sdk) — `GivenDotnetBuildInvocation.cs`
> - Target Runtime: Zed DAP Extension (`wasm32-wasip2`, `zed_extension_api = "0.7.0"`, WIT `dap.wit`).

---

## 1. Architectural Rationale & Upstream Mapping

In VSCode (`dotnet/vscode-csharp`), C# debugging is orchestrated by a Node.js extension host that bundles platform-specific debugger binaries inside platform VSIXs, resolves `.csproj` build assets and `Properties/launchSettings.json` profiles, and spawns the debugger over stdio or `pipeTransport`.

In Zed (`zed-csharp-debugger`), the extension runs inside a WASI 0.2 sandbox (`wasm32-wasip2`) and implements the four DAP entry points defined in Zed's `dap.wit`:
1. **`dap_request_kind`**: Classifies `.zed/debug.json` configurations into `Launch` or `Attach`.
2. **`dap_config_to_scenario`**: Translates Zed's debugger-agnostic `DebugConfig` (from the New Process UI or build locator) into a `netcoredbg`-compatible JSON configuration.
3. **`get_dap_binary`**: Normalizes the JSON configuration, strips extension-only diagnostic/profile keys into CLI flags or merged fields, resolves or auto-downloads the `netcoredbg` executable, injects `DOTNET_ROOT`, and returns a `DebugAdapterBinary` over stdio (`connection: None`) or TCP (`connection: Some(TcpArguments)`).
4. **`dap_locator_create_scenario` & `run_dap_locator`**: Two-phase build-then-debug locator that intercepts `dotnet build` (and `dotnet test`) tasks and queries `dotnet msbuild -getProperty` to discover the compiled target assembly (`TargetPath`).

### Comprehensive Upstream-to-Zed Module Mapping (Implemented + Parity Roadmap)

| Upstream Reference | `zed-csharp-debugger` Module | Tier | Status & Parity Issues |
|---|---|---|---|
| `vscode-csharp` DAP session launcher & Zed `dap.wit` guest exports | [`src/lib.rs`](src/lib.rs) | **Tier 1: WASM Boundary** | **Implemented (FR-1..5, FR-15, FR-19)**<br>Implements `zed::Extension`, session-level binary memoization (`downloaded_binary`), and `Worktree` / `Command` / `download_file` I/O bridges. |
| `netcoredbg/src/protocols/vscodeprotocol.cpp`<br>`netcoredbg/src/main.cpp`<br>`vscode-csharp` `coreclr` launch schema & `launchSettings.json` | [`src/config/`](src/config/) | **Tier 2: Pure (`std` + re-exported `serde_json`)** | **Implemented (FR-6..13, FR-18, FR-20..24)**<br>Parity extensions: `${ZED_PICK_PID}` (#8), `.vscode/launch.json` `coreclr` & `${workspaceFolder}` (#9), `Properties/launchSettings.json` profile merging (#10), `envFile` dotenv loader (#11), `console` option handling (#12), `pipeTransport` extraction (#13). |
| `vscode-csharp` debugger binary resolver & remote `pipeTransport` | [`src/resolve.rs`](src/resolve.rs) | **Tier 2: Pure (`std` only)** | **Implemented (FR-1..3, FR-9, FR-25)**<br>Parity extensions: `pipeTransport` container/SSH wrapper (`resolve_pipe_transport_command`) & TCP connect support (#13). |
| `Samsung/netcoredbg` GitHub Releases asset matrix & Zed `wasm_host/wit/since_v0_8_0.rs` (`download_file`) | [`src/download.rs`](src/download.rs) | **Tier 2: Pure (`std` only)** | **Implemented (FR-16)**<br>Platform `(HostOs, HostArch)` to release asset mapping, macOS `x86_64` fallback tag (`3.1.3-1062`), cache directory layout (`netcoredbg/<version>/netcoredbg/netcoredbg[.exe]`), numeric-aware version comparison (`compare_versions`, `newest_version`), and stale version pruning (`stale_versions`, #17). |
| `dotnet/sdk` `GivenDotnetBuildInvocation.cs` & MSBuild `-getProperty` JSON evaluator | [`src/locator/`](src/locator/) | **Tier 2: Pure (`std` + re-exported `serde_json`)** | **Implemented (FR-17, FR-26)**<br>Quotes-aware `dotnet build` / `dotnet test` command-line tokenizer, CLI switch-to-MSBuild property translator (`-c`, `-f`, `-r`, `--ucr`, `-o`, `--artifacts-path`, `-p:`), MSBuild JSON output parser (`parse_msbuild_properties`).<br>Parity extension: `dotnet test` Microsoft.Testing.Platform & VSTest recipes (`prepare_test_task_for_build`, #14). |
| `vscode-csharp` `package.json` `configurationAttributes` | [`debug_adapter_schemas/netcoredbg.json`](debug_adapter_schemas/netcoredbg.json) | **Static Schema** | **Implemented (FR-14, FR-27)**<br>JSON Schema (Draft-07) validating `.zed/debug.json` launch and attach configurations in Zed (#15). |

---

## 2. End-to-End Execution Flows

### 2.1 Direct Launch / Attach Flow (`get_dap_binary`)

```mermaid
flowchart TD
    A["Zed calls get_dap_binary(adapter_name, definition, user_path, worktree)"] --> B["Validate adapter_name == 'netcoredbg'"]
    B --> C["Parse definition.config as serde_json::Value"]
    C --> D["config::adapter_logging_args(&mut json_config)<br>(Extracts & strips 'logging' object -> --engineLogging / --log flags)"]
    D --> E["config::request_kind(&json_config)"]
    E -->|Launch| F["config::normalize_launch(&mut json_config, worktree_root, dotnet_path)<br>(Applies launchSettings.json #10 & envFile #11, trims program, defaults cwd, validates args/env, injects DOTNET_ROOT)"]
    E -->|Attach| G["config::normalize_attach(&mut json_config)<br>(Coerces numeric/string processId or preserves ${ZED_PICK_PID} #8)"]
    F --> H["Binary Resolution (resolve::resolve_adapter_command_with_download)"]
    G --> H
    H -->|1. Non-blank user_path| I["Use user_provided_debug_adapter_path"]
    H -->|2. worktree.which('netcoredbg[.exe]')| J["Use PATH binary"]
    H -->|3. Auto-download (FR-16)| K["download::download_plan -> zed::download_file -> make_file_executable"]
    I --> L["Return DebugAdapterBinary { command, arguments: ['--interpreter=vscode', ...logging_args], envs, cwd: worktree_root, connection, request_args }"]
    J --> L
    K --> L
```

### 2.2 Two-Phase `dotnet build` Locator Flow (`FR-17`)

```mermaid
sequenceDiagram
    participant Zed as Zed Debugger Host
    participant Ext as NetcoredbgExtension (lib.rs)
    participant Loc as locator/ (Pure)
    participant MSB as dotnet msbuild CLI

    Note over Zed,Loc: Phase 1: Task Matching
    Zed->>Ext: dap_locator_create_scenario(build_task)
    Ext->>Loc: locator::dotnet_build_args(command, args)
    Loc-->>Ext: Some(build_args)
    Ext->>Loc: locator::prepare_test_task_for_build(command, args)
    Ext-->>Zed: Some(DebugScenario { adapter: "netcoredbg", build: TaskTemplate })
    Note over Zed: Zed executes the user's `dotnet build` task
    Note over Zed,MSB: Phase 2: Artifact Discovery
    Zed->>Ext: run_dap_locator(locator_name, resolved_build_task)
    Ext->>Loc: locator::parse_dotnet_build_args(&build_args)
    Loc-->>Ext: ParsedDotnetBuild (target, configuration, framework, runtime, ...)
    Ext->>Loc: locator::msbuild_query_args(&parsed, cwd)
    Loc-->>Ext: ["msbuild", target, "-nologo", "-getProperty:TargetPath;..."]
    Ext->>MSB: Command::new("dotnet").args(query_args).output()
    MSB-->>Ext: JSON stdout {"Properties": {"TargetPath": "...", "MSBuildProjectFullPath": "..."}}
    Ext->>Loc: locator::parse_msbuild_properties(&stdout)
    Ext->>Loc: locator::resolve_program(&properties, cwd)
    Loc-->>Ext: ResolvedProgram { program: TargetPath, cwd: project_dir }
    Ext-->>Zed: DebugRequest::Launch { program, cwd, args: [], envs: [] }
    Zed->>Ext: dap_config_to_scenario(DebugConfig)
    Ext-->>Zed: Final DebugScenario
```

---

## 3. Detailed Subsystem Specifications & Parity Blueprints

### 3.1 `netcoredbg` Protocol & Configuration Dialect (`src/config/`)

Verified against `Samsung/netcoredbg`:
- **`src/main.cpp`**:
  - `--interpreter=vscode` selects the VSCode DAP protocol over stdin/stdout.
  - `--engineLogging` or `--engineLogging=<path>` logs DAP/engine traffic.
  - `--log` or `--log=<path>` writes internal debugger logs (`"stdout"` is rejected by `config/` because writing internal logs to stdout corrupts the stdio DAP wire stream!).
- **`src/protocols/vscodeprotocol.cpp`**:
  - **Launch**: Requires `"program"` (string). Optional `"cwd"` (defaults to `worktree.root_path()`), `"args"` (array of strings), `"env"` (object of string → string), `"stopAtEntry"` (bool), `"justMyCode"` (bool), `"enableStepFiltering"` (bool). Unknown fields (such as `"sourceFileMap"`) are preserved verbatim for forward compatibility.
  - **Attach**: Requires `"processId"` (normalized to a JSON integer; accepts integer `1234` or numeric string `"1234"`).
  - **`DOTNET_ROOT` Injection (FR-9)**: When `worktree.which("dotnet")` finds `/path/to/dotnet`, its parent directory `/path/to` is injected as `DOTNET_ROOT` into both the debuggee's `env` object and the adapter process's `envs` vector (unless the user already defined `DOTNET_ROOT`, checked case-insensitively on Windows).
- **Parity Extension Blueprints in `config/` (Issues #8, #9, #10, #11, #12)**:
  - **`Properties/launchSettings.json` Profile Merger (#10, FR-22)**:
    - Triggered when `"launchSettingsFilePath"` or `"launchSettingsProfile"` is present in launch config (or when `Properties/launchSettings.json` exists next to the project).
    - Pre-strips `//` and `/* */` comments and trailing commas (JSONC), then parses `"profiles"`.
    - Selects the profile named by `"launchSettingsProfile"`, or the first profile where `"commandName" == "Project"`.
    - Merge precedence (explicit launch config always wins):
      1. `profile.environmentVariables` merged into `config.env` (existing `config.env` keys take precedence).
      2. `profile.applicationUrl` injected as `config.env["ASPNETCORE_URLS"]` if `ASPNETCORE_URLS` is not already set.
      3. `profile.commandLineArgs` split with `locator::split_command_line` into `config.args` if `config.args` is absent.
      4. `profile.workingDirectory` applied to `config.cwd` if `config.cwd` is absent.
    - Strips `"launchSettingsFilePath"` and `"launchSettingsProfile"` before forwarding to `netcoredbg`.
  - **`envFile` Loader (#11)**:
    - Reads `"envFile"` (string or array of strings, resolved relative to `worktree_root`), parses `KEY=VALUE` lines (supporting `export `, quotes, and `#` comments), and merges into `config.env` beneath explicit `config.env` entries. Strips `"envFile"`.
  - **`.vscode/launch.json` `coreclr` Compatibility (#9, FR-21)**:
    - Translates `.vscode/launch.json` `coreclr` configs via `translate_launch_json` & `substitute_macros`, expanding `${workspaceFolder}` / `${workspaceRoot}` to `worktree_root`, mapping `${command:pickProcess}` → `${ZED_PICK_PID}`, and merging missing fields.
  - **`console` & `pipeTransport` (#12, #13, FR-24, FR-25)**:
    - `normalize_console` accepts/strips `"internalConsole"` and rejects `"integratedTerminal"` / `"externalTerminal"` with an actionable attach workaround.
    - `adapter_pipe_transport` extracts `pipeTransport` (`pipeProgram`, `pipeArgs`, `debuggerPath`) so `resolve_pipe_transport_command` can wrap remote/container `netcoredbg` invocations, while `tcp_connection` uses `zed::resolve_tcp_template`.

### 3.2 Automatic Download & Cache Layout (`src/download.rs`)

- **Asset Matrix (`asset_name(HostOs, HostArch)`)**:
  - `(Windows, X86_64)` → `"netcoredbg-win64.zip"` (`ArchiveKind::Zip`)
  - `(Mac, Aarch64)` → `"netcoredbg-osx-arm64.zip"` (`ArchiveKind::Zip`)
  - `(Mac, X86_64)` → `"netcoredbg-osx-amd64.tar.gz"` (`ArchiveKind::GzipTar`, fallback release tag `"3.1.3-1062"` via `fallback_tag` because Samsung stopped publishing Intel macOS binaries after 3.1.3)
  - `(Linux, X86_64)` → `"netcoredbg-linux-amd64.tar.gz"` (`ArchiveKind::GzipTar`)
  - `(Linux, Aarch64)` → `"netcoredbg-linux-arm64.tar.gz"` (`ArchiveKind::GzipTar`)
- **Zed `download_file` Extraction Contract (`download_plan`)**:
  - For `Zip` and `GzipTar`, Zed's `download_file(url, dest, kind)` treats `dest` as the **extraction directory**.
  - Because Samsung's archives contain a top-level `netcoredbg/` folder, extracting into `netcoredbg/<sanitized_version>` places the executable at:
    `netcoredbg/<sanitized_version>/netcoredbg/netcoredbg` (or `.exe` on Windows).
  - **Resilience & Cache Fallback**:
    1. Session memoization (`NetcoredbgExtension::downloaded_binary`) avoids hitting GitHub API more than once per Zed session unless `ZED_NETCOREDBG_FORCE_UPDATE` is truthy (`1`, `true`, `yes`).
    2. If the target version's binary already exists on disk, it is reused without network download.
    3. After a successful download, `stale_versions` removes older version directories.
    4. If GitHub API lookup or download fails (e.g., offline or rate-limited), `newest_version` selects the highest cached version on disk using numeric-aware version comparison (`compare_versions`).

### 3.3 `dotnet build` CLI-to-MSBuild Translator (`src/locator/`)

Verified against `dotnet/sdk` (`GivenDotnetBuildInvocation.cs`) and .NET SDK 9:
- **Forwarded Switches**:
  - `-c` / `--configuration <val>` → `-property:Configuration=<val>`
  - `-f` / `--framework <val>` → `-property:TargetFramework=<val>`
  - `-r` / `--runtime <val>` → `-property:RuntimeIdentifier=<val>`
  - `--ucr` / `--use-current-runtime` → `-property:UseCurrentRuntimeIdentifier=True`
  - `-o` / `--output <dir>` → `-property:OutputPath=<resolved_abs_dir>/` (with trailing slash) + `-property:_CommandLineDefinedOutputPath=true`
  - `--artifacts-path <dir>` → `-property:ArtifactsPath=<resolved_abs_dir>`
  - `-p:<k>=<v>` / `-property:<k>=<v>` / `--property:<k>=<v>` → forwarded as `-property:<k>=<v>`
- **MSBuild Property Query**:
  - Invokes `dotnet msbuild <target> -nologo -getProperty:TargetPath;TargetFramework;TargetFrameworks;MSBuildProjectFullPath ...`
  - Requesting multiple properties causes MSBuild to output a deterministic JSON object `{"Properties": { ... }}` without compiling anything.
  - **Multi-Target Detection**: If `TargetPath` is empty and `TargetFrameworks` is non-empty (e.g., `net8.0;net9.0`), `resolve_program` returns an actionable error instructing the user to add `-f <framework>` to their build task.
  - **Solution Rejection**: If the target ends with `.sln`, `.slnx`, or `.slnf` (or MSBuild emits `MSB1063`), returns an error advising the user to point the build task at a specific `.csproj` instead of a solution.
