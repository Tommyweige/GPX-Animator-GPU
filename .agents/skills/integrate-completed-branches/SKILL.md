---
name: integrate-completed-branches
description: "Use only when the user explicitly invokes this skill and provides a required branches list to safely integrate those selected completed branches into a base branch from the original Local checkout; accept optional base_branch (default main), push (default false), and cleanup (default false); never auto-scan or merge all codex/* branches."
---

# 整合已完成 Branches

## 目的與輸入契約

在原始 Local checkout 中，將使用者明確指定且已完成提交的 branches，透過唯一暫時 integration/<日期>-<唯一識別碼> branch 安全整合到 base branch。這個 Skill 只負責工作流程與驗證，不會因一般 Git、branch 或 merge 對話而自動啟動。

接受使用者在提示詞中提供的以下輸入：

- branches：必填，明確指定的 branch 清單；不得從 repository 自動推導候選 branches。
- base_branch：選填，預設為 main。
- push：選填，預設為 false；只有 true 才允許在最終驗證後 push。
- cleanup：選填，預設為 false；只有 true 才允許刪除已成功合併且已驗證的暫時 integration branch。

若 branches 缺失、為空、包含無法辨識的名稱，先停止並要求使用者提供或確認清單。只處理該清單，絕不自行掃描、排序、合併所有 codex/* branches，也不把 remote 上的其他 branches 視為使用者指定內容。

可接受的提示詞形式例如：

~~~text
Use $integrate-completed-branches with branches=[<branch-a>, <branch-b>], base_branch=main, push=false, cleanup=false.
~~~

## 不可違反的安全邊界

- 不直接在 main 或其他 base branch 上測試合併個別功能 branch；個別 branch 一律先合併到暫時 integration branch。
- 不刪除、覆蓋、還原、stash、reset 或清理不屬於本次安全整合流程的既有修改。
- 除非使用者明確指定 push=true，不執行 push。
- 預設不刪除來源 branches、不移除任何 Worktrees、不刪除 integration branch。
- 除非使用者明確指定，禁止 git reset --hard、git clean -fd、force push、改寫已分享的 Git 歷史，或以任何方式丟棄既有修改。
- 不擅自替其他 Worktree 提交未提交內容；任何未提交內容都只能由其擁有者決定如何處理。

## 工作流程

### 1. 確認原始 Local checkout

先執行並記錄：

~~~powershell
git rev-parse --show-toplevel
git rev-parse --git-common-dir
git worktree list --porcelain
git branch --show-current
~~~

確認目前路徑就是 repository 的原始主 checkout，而不是任何功能 Worktree。以 git worktree list --porcelain 的主 checkout 路徑與目前路徑比對；若目前位於次要 Worktree、路徑無法確認，立即停止，不切換、修改或刪除其他 Worktree。

### 2. 確認 repository、base、status、remote 與 Worktrees

將 base_branch 缺省為 main，確認它是有效的 local branch 或可解析的 local ref，並記錄 base 的起始 commit：

~~~powershell
git status --short --branch
git branch --show-current
git rev-parse --verify <base_branch>^{commit}
git remote -v
git worktree list --porcelain
git rev-parse <base_branch>
~~~

確認目前 repository 根目錄、主要分支、Git status、remote 與完整 Worktree 清單。若原始 checkout 目前不在 base_branch，只在工作目錄乾淨且 base branch 可驗證時，才可在原始 checkout 切換到 base_branch；不得切換其他 Worktree。

### 3. 阻止 dirty 工作目錄

若目前原始 checkout 有任何未提交修改、未追蹤檔案、conflict state 或 merge state，立即停止整合並回報。不得覆蓋、丟棄、stash、還原或自行提交這些修改。

### 4. 驗證使用者指定的 branches

逐一處理 branches 清單中的名稱，不加入清單外的 branch。對每個 branch 執行：

~~~powershell
git show-ref --verify refs/heads/<branch>
git rev-parse <branch>^{commit}
git rev-list --count <base_branch>..<branch>
git diff --name-status <base_branch>...<branch>
git log --oneline --decorate <base_branch>..<branch>
~~~

確認每個指定 branch：

- branch 確實存在且能解析到 commit。
- 相對 base_branch 有實際 commits。
- 相對 base_branch 有實際檔案變更；沒有變更的 branch 列為未整合，不建立假合併。
- 不等於 base_branch、暫時 integration branch 或另一個重複輸入。

從 git worktree list --porcelain 找出該 branch 對應的 Worktree。若存在，使用 git -C <worktree-path> status --short --branch 檢查其工作目錄；只要有未提交修改，就將該 branch 列為「未整合：對應 Worktree dirty」，不得自行提交或整合其未提交內容。若 branch 沒有對應 Worktree，記錄「無對應 Worktree 可檢查」，並只整合已提交的 branch ref。

若 branch 的提交尚未完成、branch ref 不存在、沒有相對 base 的 commits 或沒有實際變更，將它列為未整合與原因。只要仍有至少一個可安全整合的 branch，可在最後報告中保留這些未整合項目；若沒有任何可安全整合的 branch，停止且不建立 integration branch。

### 5. 閱讀目的、差異與相依關係

在任何 merge 前，逐一閱讀每個可整合 branch 的：

~~~powershell
git log --stat --decorate <base_branch>..<branch>
git diff --stat <base_branch>...<branch>
git diff <base_branch>...<branch>
git diff --name-only <base_branch>...<branch>
~~~

理解每個 commit 的目的、實際程式碼行為、共同修改檔案、資料格式或 API 相依關係、建置邊界與測試涵蓋範圍。計算各 branch 的修改檔案交集；對共同檔案、上下游 API、schema、設定或資料遷移先建立整合順序。根據功能相依關係決定順序，不以 branch 名稱字母順序代替判斷；若無法安全判定順序，先停止並回報，不猜測。

### 6. 建立唯一暫時 integration branch

再次確認原始 checkout 乾淨且 base_branch 的 commit 仍等於步驟 2 記錄的起始 commit。從目前 base_branch 建立唯一暫時 branch，格式必須是：

~~~text
integration/<日期>-<唯一識別碼>
~~~

例如使用當地日期與隨機或 UUID 的短識別碼；建立前確認同名 ref 不存在。可在原始 checkout 使用等價流程：

~~~powershell
$date = Get-Date -Format yyyyMMdd
$id = [guid]::NewGuid().ToString('N').Substring(0, 8)
$integration_branch = "integration/$date-$id"
git switch --create $integration_branch <base_branch>
~~~

記錄 integration branch 名稱與 base 起始 commit。此後所有個別 branch 的測試合併都在 integration branch 上進行，絕不直接在 main 或其他 base branch 上測試個別功能 branch。

### 7. 逐一合併與逐次驗證

依步驟 5 決定的順序，一次只合併一個可整合 branch，保留清楚的 branch 邊界：

~~~powershell
git merge --no-ff <branch>
~~~

每次只在該次 merge 成功後，依實際修改範圍執行相關格式化、lint／靜態檢查、建置與測試。此 repository 已由根目錄 AGENTS.md 與專案檔案驗證的可用指令包括：

~~~powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p <affected-package>
cargo build --release -p gpx-animator-native
Push-Location legacy-web
npm test
Pop-Location
~~~

只執行與該 branch 修改範圍相符的 targeted tests，但不得把未執行的檢查宣稱為通過。每次驗證後記錄 command、exit code、測試數量、ignored tests、輸出 artifact 與失敗原因；確認 git status 沒有意外產生的未追蹤或未提交檔案。

### 8. 衝突處理

發生衝突時：

1. 閱讀雙方的 commits、diff、衝突區域上下文與功能目的。
2. 讀取共同修改檔案的完整程式碼及其相依呼叫端。
3. 保留雙方仍有效的行為，以最小必要修改解決衝突。
4. 不得直接以 ours 或 theirs 覆蓋整個檔案，也不得用整檔 checkout 逃避理解衝突。
5. 解決後執行 git diff --check、相關格式化、lint、建置與測試，再完成 merge。

若目前 branch 的合併無法安全完成，執行安全中止並保留先前成果：

~~~powershell
git merge --abort
git status --short --branch
~~~

確認 integration branch 回到該 branch merge 前的狀態，保留先前已成功整合並驗證的內容；不得破壞 base_branch。停止後續 merge，將目前 branch 列為未整合並回報原因，除非使用者明確要求重新評估。

### 9. integration branch 的完整驗證

所有可整合 branches 完成後，仍留在 integration branch，依序執行完整格式化驗證、lint、建置與測試：

~~~powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo build --release -p gpx-animator-native
Push-Location legacy-web
npm test
Pop-Location
~~~

若修改範圍涉及原生輸出與環境具備 NVIDIA RTX、NVENC driver、Windows 及必要 SDK headers，再執行 README 指定的 ignored hardware acceptance gates；若環境不具備，明確回報未執行，不得宣稱完整硬體驗證通過。任何必要驗證失敗時，不得合併回 base branch。

### 10. 安全合併回 base branch

只有在 integration branch 的所有必要驗證通過後，才可準備合併回 base_branch：

1. 確認原始 checkout 仍乾淨，且目前仍在 integration branch。
2. 重新執行 git rev-parse <base_branch>，與步驟 2 記錄的 base 起始 commit 比對。
3. 若 base branch 在整合期間已發生變更，立即停止並重新評估；不得強制覆蓋、rebase 覆蓋或直接假設變更可安全忽略。
4. base 未變更時，切換回 base branch，執行唯一的 integration-to-base merge：

~~~powershell
git switch <base_branch>
git merge --no-ff <integration_branch>
~~~

5. 若合併回 base 發生衝突，依同一套衝突理解與最小修改流程處理；無法安全完成時使用 git merge --abort，保留 base branch 原狀並回報。
6. 合併回 base 後，再次執行完整格式化驗證、lint、建置與測試套件；測試失敗時不得宣稱 base 已完成整合。

### 11. Push 與 cleanup

預設 push=false、cleanup=false：

- 不 push 任何 branch。
- 不刪除來源 branches。
- 不移除來源或其他 Worktrees。
- 不刪除 integration branch。

只有 push=true 且 base 合併後的完整測試通過時，才可將 base branch push 到其已設定的 non-force upstream。若沒有 upstream、remote 不明確、需要 force push 或 push 前發現 base 又變更，停止 push 並回報，不自行猜測 remote。

只有 cleanup=true、base 合併成功、最終測試通過且 integration branch 已不再被 checkout 時，才可刪除該暫時 integration branch。cleanup=true 不授權刪除來源 branches 或移除 Worktrees；除非使用者另行明確要求，仍保留它們。

## 最後回報

完成、部分完成或安全中止時，明確回報：

- integration branch 名稱。
- base branch。
- 實際合併順序。
- 已整合 branches。
- 未整合 branches 與每個原因。
- 發生及解決的衝突；若無衝突也要明確寫出。
- 執行的格式化、lint、建置與測試、各自結果，以及 skipped／ignored 項目。
- base branch 最終 commit；若未合併則回報仍維持的 commit。
- 是否還有未提交修改。
- 是否有執行 push 或 cleanup，以及實際結果。

若任何必要步驟未執行、失敗或因環境限制跳過，清楚標示原因，不得宣稱整合或測試成功。
