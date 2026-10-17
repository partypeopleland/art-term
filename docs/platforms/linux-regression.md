# Linux 官方 Runtime 與 MCP bridge 驗收

Linux 測試用來補上 App 本機協定 fixture 無法證明的部分：官方 openab-pty 是否真的建立 PTY、執行 shell、恢復原 session，以及真截圖 helper 是否能把圖片送回。它與 [Windows E2E](windows-regression.md)、[APK E2E](android-regression.md)共同構成[測試矩陣](../refarch/test-matrix.md)，不要求另做一套 Linux GUI。

使用者需要一台可由 AI 操作的 Linux x86_64 主機、一般使用者帳號、專案 checkout、Python 3.10 以上及 Bash／基本 Linux 工具。官方 binary 直接取得並核對固定 digest；**跑 Host／Runtime 測試不需要 Rust、Cargo 或 Docker**。只有要在 Linux 編譯 App 共用核心時，才另需 Rust／Node 開發工具。

## 直接 MCP 的新驗收路徑

新版 App 截圖直接走原生 MCP HTTP，受控端 adapter 只負責 AI 工具。正式驗收使用固定 client 經產品 stdio adapter→真 A／B 測試 MCP，另經 Rust configure→官方 Runtime 管理 PTY 驗證 READY／關 echo、秘密不進 history／replay／cmdline／environ／log、Shell 清理與 renew 後重新 configure／clear。CLI 參數另用契約測試，不能宣稱真 Codex／AI 已測。完整門檻見[測試矩陣](../refarch/test-matrix.md#直接-mcp-與獨立-screens-的驗收門檻)。

以下既有 runner／helper 流程是舊版相容驗收，不是新版 App 的截圖路徑；其歷史結果不能當成直接 MCP 已通過。實際新測試入口與結果以當次交付的測試檔及報告為準，不從舊版通過紀錄推定。

## 測試分層

```text
Isolated Linux test process
   +--> pinned official openab-pty --> real work PTY
   |                              +--> dedicated screenshot PTY
   |                                           |
   |                                     real preview.py
   |                                           |
   |                                     real mcp_preview.py
   |                                           |
   |                            local MCP fixture / valid PNG
   |
   +--> host installer/upgrade/desktop fixture suites
   +--> optional existing MCP screenshot-only check
```

Runtime 使用新的 HOME、TMPDIR、credential、loopback port 與程序；helper 放在這份 HOME 內，沿用舊版 App 的固定呼叫方式。有效大 PNG 經真 MCP helper、framing、PTY 與 WebSocket 傳回，再核對 byte、CRC、解壓資料與圖片大小。工作 Shell 與截圖 Shell 分開，清理只刪自己建立的 session。

預設 MCP 是本機測試服務，這能驗證真 helper 和 Runtime 的整合，不能證明現場的共享桌面服務可用。選配真 MCP 檢查只沿用既有受保護設定擷取一次畫面，不啟動／重啟服務，不開瀏覽器、不變更共享桌面；結果只保留大小與 hash，圖片及 token 不進 Git。真 MCP 的單次擷取檢查也不代表持續可用或已驗證完整 App 到正式網路的路徑。

Host 安裝／升級／桌面交易測試以假的 systemd／Tailnet adapter 操作隔離檔案，其中安裝流程會啟動真的官方 binary。它們能重現拒絕條件與復原，不能冒稱正式 systemd／Serve 部署已驗收。

## AI 準備與執行

先確認主機、一般使用者、checkout、分支、commit 與 dirty/untracked，沿用同一已發布測試分支。Windows／NUC 不同時修改同一檔案；Git 同步只搬原始碼與文件，不搬私人設定。不能拿舊主機 checkout 的結果代表新來源。

從專案根目錄執行：

```bash
python3 scripts/linux-regression/run.py --host-fixtures
```

未指定 binary 時，腳本依專案固定的官方 artifact metadata 取得執行檔並核對 digest。已有相符官方 binary 時可指定絕對 `--binary` 路徑，仍會核對大小與 hash，不接受任意聲稱官方的執行檔，也不修改原檔。

需要驗證已準備好的真 MCP，可增加 `--live-mcp-config /absolute/private/desktop.env`。該檔必須是目前使用者擁有、其他帳號不能讀寫的 regular file，使用該舊 runner 接受的受保護設定格式；新版 App 不讀這份設定。不要將 token 寫入命令列、對話或測試報告。

新 Runtime 動態選空閒 port，不停止既有 listener。既有安裝測試預設 port 為 18090；整合 runner 給它空閒的 `--port`。這只是 target 內白名單 installer 測試副本的 port 置換，正式 `init-host.sh` 的預設與功能不變；來源模板不符時拒絕置換。測試 HOME／TMPDIR 均在專案 target 內。

## 判定與限制

2026-10-11 在 Linux x86_64 完成真 Runtime、真 helper 大圖及一次既有 MCP 擷取；Host 四套共 53 tests 通過。首次安裝 fixture 誤用固定 readiness port，修正為隔離 port 後，定向重跑該套 16 tests 通過；這是兩輪結果的合併驗收，沒有將首次失敗改記成功。正式 installer 與 openab-pty 均未修改。

結果留在 `target/linux-regression/run-*/result.json`，記錄真官方 binary、各案例、選配真 MCP 是否執行及清理狀態。驗收至少確認實際 shell Unicode／resize、原 token 與游標恢復、大圖 helper 通道及工作隔離、自建 session 回收、Runtime 程序結束；加上 `--host-fixtures` 時各套測試都必須成功。退出非零、缺結果或 cleanup 失敗均不能記 PASS。

失敗時保留不含秘密的結果，修正測試或產品的實際原因後再跑受影響案例。不能刪 journal、停正式服務、占用別人的 port 或放寬圖片驗證來取得通過。正式部署與復原仍依 [Host 維護](linux-host-maintenance.md)，Windows／APK 操作與實機驗收仍各自適用。

## 附錄：實作與參考

[Linux runner](../../scripts/linux-regression/run.py)、[官方 artifact](../../scripts/openab-runtime/official_artifact.py)、[安裝測試](../../scripts/test-init-host.py)、[升級測試](../../scripts/test-openab-upgrade.py)、[桌面交易測試](../../scripts/test-openab-desktop.py)、[helper 測試](../../scripts/test-openab-official.py)、[真 helper](../../scripts/openab-runtime/mcp_preview.py)。
