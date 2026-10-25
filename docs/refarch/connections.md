# Runtime 與 Screen 連線設定

`connections.toml` 是唯一連線來源。Windows 預設讀寫 EXE 旁的檔案，也可用 `art-term-gui.exe --config C:/path/connections.toml` 明確指定；Android 讀寫 `<noBackupFilesDir>/openab-operator-v1/connections.toml`。不讀取使用者目錄的舊 Host JSON，不從嵌入 catalog 或 bootstrap 補入連線。

## 一份檔案如何成為清單

```text
App startup -> connections.toml
                     |
                     +-> missing / empty / comments / [] -> empty list
                     +-> valid data -> HostStore snapshot -> UI list
                     +-> invalid / unreadable / decrypt error -> report error

Add / edit / import -> validate + platform seal
                              |
                    check loaded file fingerprint
                              |
                    temp file -> sync -> atomic replace
                              |
                    switch in-memory snapshot
```

HostStore 是原生層持有的 Runtime 與 Screen 兩個獨立集合；snapshot 是某次讀取的固定狀態。啟動只讀取檔案，空來源就是空清單，沒有另一份隱藏 catalog 補回主機。新增、編輯或匯入先完成驗證與憑證保護，再核對載入時的檔案指紋，最後原子替換；只有保存成功才更新記憶體。如此可讓介面中的名稱、分類、能力與憑證始終來自同一份來源。

## 第一次使用與重設

檔案不存在、空白、只有註解，或只有 `connections = []` 時，App 正常開啟空清單。可新增第一筆連線，或在全站齒輪的「連線設定」選擇匯入。啟動不建立或改寫檔案；第一次成功新增／匯入 Host 後才建立 schema 5 TOML；有 Screen 時寫 schema 6。Host CRUD／匯入不得遺失 Screen 集合，刪除最後一筆 Host 也不刪 Screen。

Windows 請放在可寫目錄。手動修改或刪除檔案前先關閉 App，再重新開啟；刪除後就是全新空清單。執行期間檔案被外部變更時，舊 snapshot／保存操作會拒絕，避免將記憶體中的舊設定寫回。沒有檔案監看或跨程序寫入鎖，同一檔案只開一個 App。

非空但格式錯誤、讀取失敗或憑證解密失敗會回報錯誤並保留原檔，不當成空清單，也不改讀別處。保存使用同目錄暫存、同步與原子替換，成功後才切換記憶體。

## 連線與能力

App 新增連線可選 Computers／Agents／Screens；前兩者保存 Host／Agent Runtime 設定，Screen 使用下節獨立模型。Runtime 表單填寫名稱、可信 HTTPS 根網址與管理憑證。最多 64 筆、整檔最多 1 MiB；名稱 1–128 UTF-8 bytes，不含控制字元；管理憑證最多 1024 UTF-8 bytes，不含空白／控制字元。origin（HTTPS 的 scheme、主機及 port 所組成的入口）不重複，不能有帳密、額外路徑、query 或 fragment，不支援略過 TLS 驗證。

沒有 Screen 時 App 寫出的結構為 schema 5；以下為不含憑證的手寫範例：

```toml
schema_version = 5
[[connections]]
id = "example-host"
revision = 1
name = "Example Host"
openab_pty_url = "https://runtime.example.test:8443"
category = "host"
desktop_transport = "runtime-mcp"
coding_cli = { id = "assistant", name = "Assistant", command = "assistant-cli", args = ["--model", "example-model"] }
```

id 為舊版小寫 slug（1–32 個 ASCII 小寫字母／數字／非首字元連字號），或 App 產生的 canonical UUID。id 唯一，revision 為正整數，用來辨識同一筆設定的版本。category 為 host 或 agent，只決定分類；desktop_transport 與 coding_cli 均可省略，不從分類推斷能力。舊 desktop_transport 保留解析，只供未選 Screen 的 Windows 舊分頁 Runtime 截圖相容；不控制 Android Screen 入口或明確選取後的直接 MCP 路由。更新沿用 id、增加 revision；未知欄位拒絕，錯誤不回印原輸入。

每筆連線可設定一個 coding_cli：id 為上述 slug、name 最多 80 字元、command 最多 256 UTF-8 bytes 且不以 `-` 開頭；最多 32 個 args，各 ≤1024 bytes，command 與 args 合計 ≤8192 bytes，皆拒絕控制字元。使用者在已連線 Shell 明確點擊後，才逐參數 POSIX quoting 送出；重連不自動執行，App 不安裝或在本機執行 CLI。

metadata 直接來自這份 TOML。新增／編輯 Runtime 表單修改分類並保留既有相容欄位與 CLI 能力。target（後端核發的連線目標）發出後凍結 Runtime 與 CLI 能力，後續設定更新不改綁既有分頁；Screen 由使用者在分頁另行明確選擇，不由 Host metadata 推導。

## 獨立 Screens 集合

Screen 保存於同一檔案的 `desktops` 集合，欄位是 id、revision、name、mcp_url、token_ciphertext；沒有明文 token 欄位。它不是 category=screen 的 Runtime Host，也不需要 openab-pty URL。全站設定提供新增、編輯、刪除；Main 的 Screens 顯示已保存清單與數量。建立 Host／Shell 不必先有 Screen，刪 Host 也不刪 Screen。

MCP URL 可包含服務路徑，例如虛構的 `https://screen.example.test/mcp`，不得含帳密、query、fragment。HTTPS 驗證正常 TLS；HTTP 只允許 loopback 或 Tailnet 範圍，DNS 每次連線重新核對並固定到合格位址，詳見[傳輸規則](desktop.md#原生-mcp-傳輸)。token 是最多 1024 bytes 的可見 ASCII，保存使用獨立 domain、id、URL、revision 的 CredentialCipher context，沿用平台金鑰但不與 Runtime 憑證互換。

同 URL 編輯可不重填 token，改 URL 必須重填；revision 更新後重新 seal。讀回介面只提供非秘密 metadata，不回填密文內的 token。設定刪除、修改或檔案外部變更後，過期 snapshot 不得重新啟用預覽或工具。Host CRUD 與僅 Host 的匯入保留完整 Screen 集合；[連線搬移](connection-transfer.md)亦可選取獨立 Screens，含 token 時整份密碼加密，目的裝置重新保護。

## 憑證保存

`credential_ciphertext` 是 App 寫入的非空 byte array（最多 32768 bytes），與名稱、能力保存於同一檔案。Windows 用 current-user DPAPI、Android 用原 operator Keystore 保護，沒有明文憑證欄位。檔案 schema 5／6 不改變既有 Host 的 CredentialContext schema 1、id／origin／revision／AAD 或既有平台金鑰。

省略密文的手寫連線會顯示「缺少管理憑證」，可刪除、編輯或匯出不含憑證的設定；連線前必須補入憑證。同 origin 編輯可保留已有憑證，改 origin 必須重新輸入；密文不應手動修改或跨裝置複製。輸入在提交、取消、失敗、關頁與背景後清除，不回填已儲存憑證。

跨裝置使用[連線匯出／匯入](connection-transfer.md)：密碼保護搬移檔案，匯入後重新以目的裝置的 DPAPI／Keystore 保護並寫回唯一 TOML。沒有即時同步。平台資料位置與金鑰連續性見[執行時檔案](credential-storage.md)。

## 舊設定與版本升級

有界接受 schema 2／3／4 的既有非秘密 TOML，轉為同一連線模型、revision 1、缺少憑證；啟動不改檔，成功保存時沒有 Screen 寫 schema 5，有 Screen 寫 schema 6。公開空設定與虛構範例仍是相容的 schema 4 輸入，檔案位置列於附錄。

v2 仍要求 auto_discover=false 及空 computers／agents；退役 SSH-only 設定只能按既有規則驗證後略過，不恢復 SSH。舊 endpoint.runtime_origin、runtime_agent.origin 及桌面關聯轉為原 HTTPS origin 的 Host／Agent 與 Native runtime-mcp。舊 coding_cli="codex" 只於相容解析轉為明確 CLI 設定。既有設定須同時符合 Host 名稱 byte 數界限，不截斷資料。

0.1.17 的 `runtime-hosts-v1.json`、舊 bootstrap marker 與平台金鑰保留但不自動遷移。需要沿用時，先在舊版匯出，再於新版匯入。私有 bootstrap 建置開關已退役，設定時建置會拒絕。Android 不再支援建置時嵌入連線或 `-ConfigPath`；透過 App 新增／匯入即可。

新增第一筆 Screen 前，若原 connections.toml 存在，同目錄先建立 `connections.pre-desktops.toml` 備份；Unix 權限為 0600。備份僅建立一次，已有合法私有備份時保留，不覆寫；symlink、非一般檔案或 Unix 下其他帳號可讀寫的備份會拒絕。原設定尚不存在時無舊檔可備份。已有 schema 5 啟動時不改檔，有 Screen 的保存才使用 schema 6，刪除最後一筆 Screen 後保存為 schema 5。新版本保留 Host id／revision 與既有加密 context，Screen token 使用獨立 context。備份也含平台密文，須和原檔一樣保護，不能放 Git。

降版到不支援 schema 6 的 App 前，關閉 App 並還原新增 Screen 前的備份，或先在新版明確移除 Screen 集合、確認保存為 schema 5；不能為了降版靜默丟棄桌面。還原舊備份也會回復備份後新增／修改的 Host，應先確認差異。跨裝置可匯出含 Screens 的加密搬移檔；這與降版前的完整 TOML 備份用途不同，搬移檔不保留來源平台密文或工作狀態。

## Runtime 契約

App 使用未修改的官方 openab-pty 管理 API 與 WSS，不要求本專案的相容標記、Runtime process identity 或 Shell identity 擴充。短暫 transport／timeout 最多以原 attach token 與已呈現輸出游標恢復 3 次，不自動續期、建立／重啟 Shell，也不重播輸入。TTL、接管、token 失效或無法確認的工作狀態會停止恢復，讓使用者重新選擇；官方名稱／generation 不提供舊版自訂原子身分比對。ACK 表示已解析輸出的本機確認，Gap 表示輸出缺段；Gap 提示並重置顯示，手機背景暫停恢復，回前景才繼續。

### App 與既有 Host 的版本相容

同一 App 主版本內，Android 與 Windows 升級保留既有受支援 Host 的基本 Shell 通訊，不要求使用者為每次 App 更新升級 Host。新增功能以能力宣告判斷是否可用；舊 Host 缺少新能力時，只停用該功能，保留原有列清單、建立、連線與接續行為。不得因缺少選用能力而要求整個 Host 升級。

若變更無法向後相容，必須提高 App 主版本號，明列受影響的 Host 版本、升級與遷移方式。正式發布前，回歸既有受支援 Runtime 的管理 API 與 WSS，以及缺少新增選用能力的情境。固定來源的官方 Runtime 是目前基本通訊基準；App 與 Host 的版本號不要求相同。

HTTP `502`／`504` 表示主機入口或轉送服務異常，不作為 Runtime 版本不相容的證據。App 在兩平台使用相同錯誤分類，顯示固定訊息，不顯示上游回應內文。

首次準備 Host 見[快速開始](../openab-host-setup.md)，升級見[Host 維護](../platforms/linux-host-maintenance.md)，桌面能力見[桌面預覽](desktop.md)。
## 附錄：實作與參考

[crates/art-term-core/src/connections.rs](../../crates/art-term-core/src/connections.rs)、[crates/art-term-core/src/openab/profiles.rs](../../crates/art-term-core/src/openab/profiles.rs)、[crates/art-term-gui/src/shared/runtime_hosts.rs](../../crates/art-term-gui/src/shared/runtime_hosts.rs)。

[公開空設定](../../config/connections.example.toml)、[虛構設定範例](../../config/examples/connections.toml)。
