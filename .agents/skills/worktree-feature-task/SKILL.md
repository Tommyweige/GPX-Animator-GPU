---
name: worktree-feature-task
description: "Use only when the user explicitly invokes this skill with a required task to perform one feature task inside an independent Codex-created Git Worktree; stop in the original Local checkout, never work directly on main, and never merge or integrate to main. Accept optional task_slug and base_branch (default main)."
---

# Worktree 功能開發

## 目的與輸入契約

在 Codex 建立的獨立 Git Worktree 中，安全執行單一功能開發任務，建立唯一的功能 branch，實際完成修改，依 repository 規範驗證並建立 commits，最後使用固定格式回報。此 Skill 不負責 merge、整合或回寫 main。

只在使用者明確使用 $worktree-feature-task 或明確要求啟動此 Skill 時執行；不可因一般功能討論、Git 對話或任務描述而隱式啟動。

接受使用者提供的輸入：

- task：必填，使用者要完成的功能或修改。若缺少或內容不足以界定範圍，停止並要求補充。
- task_slug：選填，用於 branch 名稱；未提供時，根據 task 產生簡短英文 slug。
- base_branch：選填，預設為 main；必須是可驗證的 local base branch。

可接受的提示詞形式：

~~~text
Use $worktree-feature-task with task="<使用者指定的功能或修改>", task_slug="<optional-slug>", base_branch=main.
~~~

不要在 Skill 內寫死特定功能 branch 名稱。功能 branch 一律依輸入產生唯一名稱，不得重用現有 branch。

## 工作流程

### 1. 讀取 repository 規範

開始任何檢查或修改前，讀取 repository 根目錄的 AGENTS.md，以及目前目錄到 repository 根目錄路徑上所有適用的 AGENTS.md。若更深層目錄存在適用規則，也要在修改該目錄檔案前讀取。

同時閱讀與 task 相關的 README、專案設定、測試說明、CI 設定與其他工作規範。根目錄 AGENTS.md 的規定優先於本 Skill 中的通用指示；本 Skill 不得放寬它對 Worktree、commit、驗證與 Git 安全的要求。

### 2. 確認目前是獨立 Git Worktree

執行並記錄：

~~~powershell
git rev-parse --show-toplevel
git rev-parse --git-dir
git rev-parse --git-common-dir
git worktree list --porcelain
~~~

用 git rev-parse --git-dir 與 git rev-parse --git-common-dir 判斷目前是否為 linked Worktree：若兩者解析到同一個 Git directory，代表目前是原始 Local checkout，必須停止。也要用 git worktree list --porcelain 比對目前路徑與主 checkout 路徑，確認目前不在原始 Local checkout。

### 3. Local checkout 停止條件

如果目前位於原始 Local checkout，停止所有操作，不建立 branch、不修改檔案、不執行產品驗證，並提醒使用者應在「新工作樹」模式啟動此任務。不得為了繼續任務而自行建立、切換、移除或重建其他 Worktree。

### 4. 開始前檢查 Git 狀態

確認 task 已提供後，檢查：

~~~powershell
git status --short --branch
git rev-parse HEAD
git branch --show-current
git worktree list --porcelain
git show-ref --verify refs/heads/<base_branch>
git rev-parse <base_branch>
git rev-list --left-right --count <base_branch>...HEAD
git diff --stat <base_branch>...HEAD
~~~

記錄目前 HEAD、目前 branch、Worktree 清單、base branch commit，以及目前 HEAD 相對 base branch 的 ahead／behind 狀態和差異摘要。不得假設目前 branch 已經等於 base，也不得忽略目前已有的 commits。

### 5. 保護不是本任務產生的修改

在開始修改前，git status --porcelain 必須為空。若目前 Worktree 存在任何未提交修改、未追蹤檔案、staged 內容、conflict state 或 merge state，視為不是本次 task 產生的修改：

- 不得覆蓋、刪除、還原、stash 或提交。
- 不得使用 git reset --hard 或 git clean -fd 清理。
- 停止操作並清楚回報檔案、狀態與停止原因。

不要透過修改其他 Worktree 或原始 Local checkout 來繞過這個停止條件。

### 6. 建立並確認唯一功能 branch

如果目前是 detached HEAD，在確認 Worktree 乾淨後，自行從目前 HEAD 建立唯一功能 branch。branch 名稱格式必須是：

~~~text
codex/<task-slug>-<唯一短識別碼>
~~~

task_slug 必須符合以下條件：

- 使用小寫英文與數字。
- 單字之間使用連字號。
- 簡短描述 task 目的。
- 不包含空白、斜線以外的特殊字元、中文或保密內容。
- 不得與任何現有 local branch 重複。

未提供 task_slug 時，根據 task 的語意產生簡短英文 slug；不要只把敏感內容原樣放進 branch 名稱。以 UUID 或隨機短識別碼避免重複，建立前使用 git show-ref --verify refs/heads/<generated-branch> 確認不存在；若衝突，重新產生識別碼，不切換到既有 branch。

可在目前 linked Worktree 執行等價流程：

~~~powershell
$task_slug = "<normalized-lowercase-english-slug>"
$id = [guid]::NewGuid().ToString('N').Substring(0, 8)
$feature_branch = "codex/$task_slug-$id"
git show-ref --verify "refs/heads/$feature_branch"
git switch --create $feature_branch HEAD
~~~

如果目前已位於 codex/* 功能 branch：

1. 確認 git branch --show-current 與 git worktree list --porcelain 都指向目前 Worktree。
2. 閱讀該 branch 相對 base 的 commits、diff 與修改目的。
3. 確認其既有內容屬於目前 task，或 branch 尚未包含其他 task 的修改。
4. 若 branch 目的不符、包含另一個未完成任務、或無法判斷歸屬，停止並回報，不任意切換到其他 branch。

如果目前是其他非 codex/* branch，也不要切換到別的 branch；除非能安全建立符合規則的唯一功能 branch，否則停止並要求使用者在正確的新 Worktree 啟動任務。任何情況都禁止直接修改或提交到 main。

### 7. Git 安全禁止事項

本 Skill 執行期間禁止：

- 直接修改或提交到 main 或其他 base branch。
- 切換到、合併到或從其他 Worktree 讀寫未授權內容。
- 切換、修改或刪除其他 Worktree。
- 刪除任何 branch 或 Worktree。
- git reset --hard。
- git clean -fd。
- force push。
- 改寫已分享的 Git 歷史。
- 丟棄任何既有修改。
- 自行 merge 或整合到 main。

完成 task 後留在功能 branch，等待使用者或專用整合流程處理後續整合。

### 8. 閱讀程式碼並制定最小實作範圍

在修改前，閱讀 task 涉及的程式碼、文件、測試、設定、相依關係與 build 入口。先找出：

- 目前行為與 task 要求的差異。
- 會受影響的模組、API、資料結構與測試。
- 可能需要同步修改的文件或設定。
- 適用的格式化、lint、靜態檢查、單元測試、整合測試與建置指令。

只修改完成 task 必要的內容，避免無關重構、無關格式變更、批次改名或相依套件更新。不要只提供操作建議或程式碼片段；必須在目前功能 Worktree 實際完成 task。

### 9. 評估共用檔案與潛在衝突

若 task 必須修改共用檔案、套件鎖定檔、入口檔、全域設定、schema、migration、build script 或 public API，先評估對其他 branches 的影響與修改順序。保持修改最小，並在最後回報標示：

- 共同修改檔案與可能衝突區域。
- 新增或變更的相依套件。
- migration、設定或資料格式變更。
- 需要其他 branch 配合的 API、介面或順序。

### 10. 實作 task 與分階段提交

實際完成 task，並在每個完整且可獨立理解的邏輯階段建立清楚 commit。每次 commit 前：

~~~powershell
git status --short
git diff
git diff --cached
~~~

只 stage 本次 task 產生且屬於該邏輯階段的檔案。若發現不屬於 task 的修改，停止並回報，不要用 reset、checkout 或清理命令處理。

commit message 使用專案既有慣例；若專案沒有既有規範，使用清楚的 Conventional Commits 格式，例如 feat: ...、fix: ... 或 docs: ...。不要把多個無關階段塞進一個模糊 commit，也不要建立空 commit。

### 11. 依實際專案設定執行驗證

commit 前依適用的 AGENTS.md 與實際專案檔案執行驗證。不得憑空猜測指令；先從 AGENTS.md、README、CI、package.json、Cargo.toml、pyproject.toml 或其他設定確認。

本 repository 目前已驗證的原生 Rust 指令包括：

~~~powershell
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo build --release -p gpx-animator-native
~~~

legacy-web 的 Node.js 測試入口來自其 package.json：

~~~powershell
Push-Location legacy-web
npm test
Pop-Location
~~~

依 task 修改範圍執行適用的：

- 格式化。
- lint／靜態檢查。
- 單元測試。
- 整合測試。
- 建置。

若 package.json 沒有 lint 或 formatter script，不得自行宣稱存在 npm run lint 等指令；應回報「專案未定義該驗證」。若 task 影響跨模組行為，除 targeted tests 外也執行專案完整測試。硬體、SDK、GPU 或外部服務限制必須明確記錄。

### 12. 驗證無法執行時

若某項驗證無法執行：

- 說明具體原因、嘗試的 command、exit code 或環境限制。
- 不得宣稱該項驗證通過。
- 判斷是否仍能安全提交目前已完成且可理解的修改。
- 若提交仍安全，commit message 與最後回報必須標示未完成或未執行的驗證。
- 若無法判斷修改是否安全，停止提交並回報，不用破壞性命令排除問題。

### 13. 任務完成收尾

任務完成後：

1. 確認所有有效修改都已提交，且 commit 只包含本次 task。
2. 執行 git status --short --branch，工作目錄應保持乾淨。
3. 若有由本次 task 產生且未被忽略的輸出檔，先以不丟棄既有內容的方式處理；無法安全處理時停止並回報。
4. 確認目前仍位於該功能 Worktree 與 codex/* branch。
5. 不自行合併到 main。
6. 預設不 push，只有使用者明確要求時才可考慮 push；本 Skill 本身不執行 push。

若工作目錄無法乾淨，或有任何有效修改未提交，不得宣稱任務完成；列出具體檔案與原因。

## 與根目錄 AGENTS.md 的相容性

本 Skill 遵守根目錄 AGENTS.md 的 Worktree、唯一 codex/* branch、保護既有修改、分階段 commit、驗證、禁止破壞性 Git 操作與固定回報要求。它只在獨立功能 Worktree 執行，不取代原始 Local checkout 的整合總管，也不執行 integration branch 或 merge main 的流程。

## 任務結果

- 任務：
- branch 完整名稱：
- 起始 base branch：
- 最終 HEAD：

## Commits

- <commit hash> <commit message>

## 修改內容

- 功能摘要
- 修改檔案清單

## 驗證結果

- 格式化：
- lint／靜態檢查：
- 測試：
- 建置：
- 無法執行的驗證及原因：

## 整合注意事項

- 可能與其他 branches 衝突的檔案或區域
- 新增或變更的相依套件
- migration／設定變更
- 尚未解決的問題

## Git 狀態

- 工作目錄是否乾淨
- 是否已 push
- 是否已合併 main
