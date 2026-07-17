# 專案工作規範

## 基本原則

* 開始任何工作前，先閱讀本檔案及相關專案文件。
* 修改前先檢查 git status、目前 branch 與既有差異。
* 不得刪除、覆蓋或還原不是目前任務產生的修改。
* 只修改目前任務必要的檔案，避免無關重構。
* 不得使用 git reset --hard、git clean -fd 或 force push。

## 多對話與 Worktree

* 每個功能開發任務使用獨立 Codex Worktree。
* 每個功能 Worktree 必須建立唯一的 codex/* branch。
* 功能 Worktree 禁止直接修改或合併 main。
* 功能 Worktree 禁止切換、修改或刪除其他 Worktree。
* 每個完整邏輯階段建立清楚的 Git commit。
* 任務完成時，所有有效修改都必須提交，並回報 branch 與 commits。

## 整合流程

* 只有原始 Local checkout 的整合總管可以整合功能 branches。
* 整合前先確認 main 工作目錄乾淨。
* 整合時先從 main 建立 integration/* 暫時分支。
* 一次只合併一個功能 branch。
* 每次合併後執行相關建置與測試。
* 發生衝突時必須理解雙方功能目的，不得直接以 ours 或 theirs 覆蓋整個檔案。
* 所有整合與完整測試通過後，才可將 integration branch 合併回 main。
* 預設不自動 push、不刪除 branch、不移除 Worktree，除非使用者明確要求。

## Parallel Worktree Coordination

These rules supplement the Worktree rules above and apply to every linked Codex feature Worktree and to the original Local checkout when it acts as the integration manager:

* At most one Worktree may run a complete build, complete test suite, release build, or large resource-generation job at any time. Treat this repository's workspace-wide Cargo checks, release build, README-listed ignored RTX acceptance gates, legacy-web executable packaging, and any large video/data generation as exclusive operations.
* A normal feature Worktree may run only checks, tests, and compilation directly related to its current task. Prefer affected-package checks and targeted tests; do not run workspace-wide or release validation from every feature Worktree.
* Complete build, complete test suite, integration tests, release validation, and final acceptance are owned by the merge/integration task. Feature tasks must leave those exclusive operations to that task unless the user explicitly assigns the exclusive validation slot.
* Before starting an exclusive operation, confirm that no other Worktree is using the slot and record the owner, Worktree path, branch, command, and start time in the task coordination channel or an external coordination record. If exclusive ownership cannot be confirmed, wait and report instead of starting the command.
* Do not start a continuous watch server, dev server, background service, or unbounded monitoring test unless the user explicitly requests it. Stop every process started for the task as soon as it is no longer needed and report any process that could not be stopped.
* Do not modify files in another Worktree, another branch, or the main working directory. Each agent may operate only on the Worktree in which it is currently running; do not use another Worktree as a build or staging location.
* Before any Git operation, confirm the current absolute path, current branch, and git status. Re-check them immediately before staging or committing so a command cannot target the wrong Worktree.
* At task start and after any interrupted or resumed conversation, inspect git status, git diff, the recent commits, untracked files, and the last recorded test state. Resume from the first incomplete step; do not restart the entire task or repeat expensive validation without evidence that it is required.
* Create a Git checkpoint after each independently verifiable phase. A clear WIP commit is allowed when useful; the merge/integration task may squash such checkpoints later.
* Never add build output, Rust target directories, temporary files, caches, exported videos, test artifacts, or large generated files to Git. Review git status and the staged diff even when a path is ignored or appears in a distribution directory.

## Parallel Scope, Recovery, and Reporting

* If the implementation plan does not match the actual code, make only the smallest adjustment required to complete the task. Stop and report before changing architecture, expanding scope, or modifying files owned by another task.
* Keep task scopes as mutually exclusive as practical. If another branch appears to modify the same core source file, shared interface, schema, Cargo manifest, lockfile, build script, or runtime configuration, report the potential conflict before making overlapping changes.
* A planning, manager, or coordination task should normally analyze, split, review, and coordinate work. It should not run a complete build, complete test suite, release build, or large resource generation, and it should not make broad product changes.
* If Codex App crashes or a task stops unexpectedly, first perform a full state inventory: confirm the current Worktree and branch, inspect git status and diff, check whether files were written only partially, determine whether tests or builds were interrupted, and find any background processes left running. Continue from the first incomplete step only after the inventory is consistent.
* Completion reports must include the modified files, test results, complete tests that were not run and why, the state of every background process started by the task, remaining risks, and the recommended merge order in addition to the existing branch, commit, and conflict reporting.

## 驗證要求

* 修改完成後執行格式化。
* 執行 lint 或靜態檢查。
* 執行相關單元測試。
* 執行專案建置。
* 整合到 main 前執行完整測試。
* 若測試無法執行，必須清楚說明原因，不得宣稱測試通過。

## 任務結束回報

每次任務完成後回報：

* branch 名稱
* commit 清單
* 修改的檔案
* 執行的測試與結果
* 尚未解決的問題
* 可能與其他 branches 衝突的區域

### Parallel Task Handoff

At task completion, also report:

* Which complete build, complete test suite, integration test, release build, and large resource-generation operations were not run, and why.
* The state of every background process started by the task, including how it was stopped or why it remains running.
* Remaining resource, validation, scope, and merge risks.
* The recommended merge order and any feature Worktree that should be merged only after another task.

## 專案工具與實際驗證指令

本專案是 Rust workspace，主要原生程式使用 Rust 1.92+、edition 2024、Windows MSVC toolchain，入口 binary 為 `gpx-animator-native`。`legacy-web/` 是獨立的 Node.js 原型，要求 Node.js 20+。目前沒有根目錄 CI 設定檔。

### 安裝相依套件

在 repository 根目錄執行 Rust 相依套件下載：

```powershell
cargo fetch --locked
```

安裝 legacy web 相依套件：

```powershell
Push-Location legacy-web
npm ci
Pop-Location
```

首次設定 Rust 環境需安裝 Rust 1.92+ 與 MSVC toolchain；README 指定 NVIDIA Video Codec SDK headers 也會在 native build 時使用。

### 格式化

Rust workspace 的格式化檢查（README 指定）：

```powershell
cargo fmt --all -- --check
```

需要套用格式化時執行：

```powershell
cargo fmt --all
```

`legacy-web/package.json` 沒有定義 JavaScript/CSS 格式化 script。

### Lint／靜態檢查

Rust workspace 的 Clippy 檢查（README 指定）：

```powershell
cargo clippy --workspace --all-targets -- -D warnings
```

`legacy-web/package.json` 沒有定義 lint 或其他靜態檢查 script；不得把不存在的 `npm run lint` 當成已驗證指令。

### 建置

原生 release build（README 指定）：

```powershell
cargo build --release -p gpx-animator-native
```

輸出檔為 `target\release\gpx-animator-native.exe`。legacy web 的可執行檔打包 script（`legacy-web/package.json`）：

```powershell
Push-Location legacy-web
npm run build:exe
Pop-Location
```

### 單元測試與完整測試

Rust workspace 單元、整合與 doc tests（README 指定的 workspace 測試）：

```powershell
cargo test --workspace --no-fail-fast
```

需要隔離執行核心 Rust package 的單元測試時，使用專案實際存在的 package 名稱，例如：

```powershell
cargo test -p gpx-core
cargo test -p scene-core
cargo test -p places-core
cargo test -p gpx-animator-native
```

legacy web 的 Node.js test runner（`legacy-web/package.json`）：

```powershell
Push-Location legacy-web
npm test
Pop-Location
```

整合到 `main` 前，至少依序執行 Rust fmt check、Clippy、workspace tests、原生 release build，以及 `legacy-web` 的 `npm test`。README 另列出僅限 RTX Windows runner 的完整硬體 acceptance gates：

```powershell
cargo test --release -p gpx-animator-native warm_cache_twenty_second_4k60_meets_realtime_gate -- --ignored --nocapture
cargo test --release -p gpx-animator-native five_minute_4k60_has_exact_frames_and_realtime_throughput -- --ignored --nocapture
cargo test --release -p gpx-animator-native ten_exports_do_not_leak_handles_or_partial_files -- --ignored --nocapture
```

若沒有 NVIDIA RTX、可用的 NVENC HEVC/H.264 driver、Windows 或必要 SDK headers，硬體 acceptance gates 或原生 build 可能無法執行；回報時必須明確標示未執行與原因。

## 本次建立依據

上述專案資訊與指令已由 `Cargo.toml`、`README.md`、`legacy-web/package.json`、`legacy-web/package-lock.json` 及 repository 目錄結構核對；未發現 CI 設定或既有根目錄 `AGENTS.md`。本規範不取代更深層目錄中未來新增的 `AGENTS.md`，如有新增則依 Codex 規則同時遵守。
