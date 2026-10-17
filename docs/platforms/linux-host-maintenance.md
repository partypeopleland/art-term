# Linux Host 進階安裝與維護

使用者先看[使用前準備](../openab-host-setup.md)，再將其中 prompt 交給自己的 AI。AI 首次安裝依[安裝操作指南](../openab-host-installation.md)執行；本頁供環境問題、指定 binary、連線排查、升級、復原或移除服務時查閱。

## 網路與服務的位置

```text
Windows / Android                    Linux Host (same Tailnet)
art-term -- HTTPS/WSS --> Serve :8443 -> openab-pty -> user Shell
    |                                                   |
    |                                              Codex stdio adapter
    |                                                   |
    +-- direct MCP screenshot --> instance-mcp <--- MCP tools
                                      |
                                  shared desktop
```

Tailnet 是 Tailscale 管理的私人網路，讓手機與 Host 互相可達；Serve 在 Host 的 `8443` 提供 HTTPS，再轉送到只監聽 loopback 的 Runtime。網路政策決定誰能到達入口，Runtime 的 admin credential（管理憑證）則決定誰能操作 Shell，兩者各自生效。Shell 以安裝者帳號執行，可存取該帳號的檔案；這個部署沒有容器沙箱。

桌面是選配：App 原生層直接連已準備的 instance-mcp，AI 工具另由工作 Shell 的記憶體 stdio adapter 連同一上游。Runtime 與 MCP 各自驗證自己的憑證；App 以平台金鑰獨立保護 Screen token，初始化腳本不代裝桌面環境。命令範例本身不授權變更既有系統服務、Tailnet 或認證；實際可操作的主機、帳號及服務以維護者已取得的授權為準。

## 確認主機與帳號

腳本支援 Linux、Bash、systemd 使用者服務，以 Ubuntu 22.04／24.04 為操作範例。預設官方產物只適用 x86_64；ARM64 必須明確提供可信且相符的未修改官方 binary。Windows、macOS、NAS 客製系統、非 systemd Linux 和容器部署不在此腳本範圍。

在主機本機終端或既有管理連線操作。登入你希望遠端 Shell 使用的一般帳號，例如 `developer`，**不要以 root 或 `sudo bash init-host.sh` 執行**。runtime、Shell 使用此帳號的 HOME，讀取正常 `.bashrc`；服務不需要 openab broker。

```bash
id -un
uname -m
ps -p 1 -o comm=
systemctl --user show-environment
```

最後一項若找不到 user bus，先透過正常登入建立此帳號的使用者 session；腳本會先檢查 user manager，`--enable-linger` 無法代替這個登入步驟。不要用 `sudo systemctl --user` 操作另一個帳號。開機無人登入仍要啟動時需啟用 linger，腳本可以代辦。若系統管理員限制 user manager／linger，請先由管理員處理；不要假造 `XDG_RUNTIME_DIR` 來跳過問題。

服務只使用一般帳號權限，`NoNewPrivileges=true` 會限制 Shell 透過 sudo／setuid 提升權限。這是 Host 上的遠端使用者終端，**沒有容器沙箱隔離**；Shell 能操作該帳號可存取的檔案。

## 找出要使用的網路資訊

在 Tailscale 管理後台的 DNS 設定確認啟用 MagicDNS（私人網路內的主機名稱解析）與 HTTPS certificates。Serve 首次啟用時，也可能提供同意網址，要求管理者完成 HTTPS 設定；腳本的 `sudo tailscale serve` 可能等你在終端輸入 sudo 密碼或到瀏覽器完成同意。設定入口與官方 Serve 指南見附錄。

查詢完整主機 DNS：

```bash
tailscale status --json | python3 -c 'import json,sys; print(json.load(sys.stdin)["Self"]["DNSName"].rstrip("."))'
```

例如得到 `workstation.example-tailnet.ts.net`。這個範例是虛構值，請用你主機實際輸出。

| 資訊 | 哪裡取得 | 是否填入 art-term |
|---|---|---|
| 完整 `*.ts.net` 主機 DNS | 上述命令／Machines | 使用它組成服務網址 |
| Tailscale IPv4 | `tailscale ip -4`／Machines | 排查連通性，通常不用填 |
| Tailnet 名稱／登入帳號 | Tailscale 後台／App | 讓裝置加入正確網路，不是服務密碼 |
| 服務 port | 初始化腳本固定 HTTPS `8443` | 包含在網址中 |
| openab-pty admin credential | 安裝腳本產生 | 後續客戶端的認證欄位 |

HTTPS 網址應使用 DNS 名稱，**不要把 DNS 換成 `100.x.x.x` IP**，否則憑證名稱不符；也不要使用忽略憑證檢查的選項。

若 Tailnet 使用自訂 ACL／grants（存取控制規則），管理者需要允許指定手機／使用者存取此 Host 的 TCP `8443`。不需要開放 loopback 的 `18090`。已登入 Tailnet 只代表裝置可參與網路，是否能到達 `8443` 仍由政策決定。

## 取得腳本與官方產物

官方 Runtime 遷移已整合 main。在 Linux Host 取得完整專案 checkout；需要重現特定交付時，再依交付紀錄使用對應 commit：

```bash
git clone --branch main https://github.com/partypeopleland/art-term.git
cd art-term
```

若已有 checkout，先保留自己的未提交內容，再切換分支；不要重置既有檔案。Windows 搬到 Linux 的腳本需保留 LF 換行。

預設以 Python 從固定官方 GHCR 映像取得 Linux x86_64 原生 binary，逐層驗證 image／config／layer／binary digest，只抽出指定執行檔。Runtime 以原帳號在主機執行，**受控端無須 Rust、Cargo、rustup 或 Docker**。ARM64 須提供可信且相符的官方 binary；腳本不會自動交叉編譯。Ubuntu 的前置工具：

```bash
sudo apt-get update
sudo apt-get install -y git curl python3 ca-certificates
```

另需系統既有的 Bash、systemd／loginctl、coreutils（含 timeout、sha256sum、install）及 util-linux（flock），以及已登入的 Tailscale。Git 只用於取得 checkout；Runtime 及 Python helper 不透過 Cargo 或 pip 安裝。所選 coding CLI 若有其他工具需求，依該 CLI 的安裝方式準備；已安裝的 Rust／Cargo 不必為本次遷移移除。

固定來源為 openab-pty commit `b82a36620a76c84a67bc6a7ab316cbc60a30e1a9` 的 `native-b82a366` 產物，以 digest 固定內容，不追蹤可變 tag。請取得完整 checkout，不能只複製 `init-host.sh`。

已有可信來源、相符 Linux 架構的未修改官方 binary 時，可明確指定。腳本驗證 SHA256 及官方設定檢查介面，manifest 記為 provided-binary；**SHA256 應來自可信來源**，自行計算只能檢查檔案是否改變，不能證明來源：

```bash
bash scripts/init-host.sh --binary /absolute/path/openab-pty --sha256 YOUR_64_HEX_SHA256 --enable-linger --serve
```

## 安裝並啟動

```bash
bash scripts/init-host.sh --enable-linger --serve
```

腳本會：

1. 檢查 Linux、帳號、systemd、Tailnet DNS、路徑與 port。
2. 啟用目前帳號的 linger，讓 user service 開機啟動、不因登出停止；此步可能要求 sudo 密碼。
3. 下載／核對 runtime，產生 admin credential，只把 SHA256 verifier 放進 runtime 設定。
4. 安裝並啟動 `art-term-openab-pty.service`，只監聽 `127.0.0.1:18090`。
5. 驗證正確認證可列出 session、錯誤認證會收到 `401`。
6. 檢查 Tailscale Serve／Funnel，新增專用的私有 HTTPS `8443` 路由到本機 runtime；此步可能要求 sudo／HTTPS 同意。

既有 `443`、其他 Serve 路由不會被重設。`8443` 已被其他路由或 Funnel 使用時會拒絕覆蓋。腳本只新增私有 Serve，不使用 Funnel。若已有完全相同的 `8443` 路由，腳本會辨識並保留。

runtime 安裝成功但 Serve 未成功時，會保留可恢復的服務與認證，並顯示錯誤。完成 HTTPS 同意／權限設定後，再執行：

```bash
bash scripts/init-host.sh --serve
```

重跑不會換密碼、重新下載或重啟既有受管服務；可以補上尚未完成的 Serve 路由。既有檔案被手動修改、權限變寬或有不明來源的同名服務時，腳本拒絕覆蓋，請先檢查。

若安裝後 linger 被關閉，可重跑 `bash scripts/init-host.sh --enable-linger`，重新啟用並驗證開機常駐；不會換認證或重啟 runtime。`--serve`／`--enable-linger` 需要可用的 sudo。`--show-info`／`--show-credential` 也會檢查服務就緒與認證，因此停止中的服務需先恢復，才能使用這兩個檢視命令。

## 管理憑證

重新查看非敏感資訊：

```bash
bash scripts/init-host.sh --show-info
```

在自己的互動終端顯示 admin credential，先存入密碼管理器，再填入 App 的 Bearer auth token 欄位：

```bash
bash scripts/init-host.sh --show-credential
```

腳本要求 stdin、stdout 都是互動終端，並將 credential 寫到 `/dev/tty`；不要將終端畫面、scrollback 或憑證貼到 issue／聊天。主機保留 owner-only、`0600` 的 credential 檔，以便安裝視窗關閉後再次取得。它與短期 attach token、Tailscale auth key及設定裡的 `sha256:` verifier用途不同。

## 確認與排查

```bash
systemctl --user status art-term-openab-pty.service
systemctl --user is-enabled art-term-openab-pty.service
loginctl show-user "$(id -un)" -p Linger
tailscale serve status
journalctl --user -u art-term-openab-pty.service -n 50
```

在另一台已加入 Tailnet 的電腦，先確認網路與 HTTPS：

```bash
tailscale ping workstation.example-tailnet.ts.net
curl --connect-timeout 5 --max-time 10 -o /dev/null -w '%{http_code}\n' https://workstation.example-tailnet.ts.net:8443/admin/sessions
```

未提供認證時，預期收到 `401`。這只能證明網路／HTTPS／認證入口，還不等於成功操作 Shell。瀏覽器開啟該 URL 收到未授權也是正常；網站不是終端客戶端。

| 現象 | 處理 |
|---|---|
| Tailnet 非 Running | `sudo tailscale up`，檢查裝置核准與金鑰期限 |
| 沒有 `*.ts.net` DNS | 檢查 MagicDNS、HTTPS 與目前登入的 Tailnet |
| HTTPS／Serve 不可用 | 完成同意網址，核對 policy、`tailscale serve status`，再跑 `--serve` |
| `8443` 衝突 | 保留既有服務；請管理者規劃不同入口，腳本不替你拆掉它 |
| 本機 runtime 未就緒 | 查看上述 user journal／status；既有服務可自行 `systemctl --user restart art-term-openab-pty.service`，重啟會結束原 session |
| CLI 指令找不到 | 新 Shell 讀 `.bashrc`；自行確認 PATH／CLI。service 保留 `.local/bin`、`.cargo/bin` 搜尋路徑以相容既有工具，不表示必須安裝 Cargo |
| credential 遺失／懷疑外洩 | 先停服務／撤除專用入口，再進行明確的重新初始化；不可把 verifier 當密碼，也不能只改 credential 檔 |

session 預設最多四個、絕對期限 12 小時、離線 idle 8 小時，輸出 ring 1MiB，attach token 15 分鐘。更新 token 不會延長原 session 的絕對期限。runtime／Host 重啟會失去 session；此腳本不是永久工作保留工具。上游的程序終止是 best effort，Host 的 systemd control group 提供服務停止時的額外清理，仍不代表任意程序逃逸都不可能。

## 檔案、升級與移除

| 路徑 | 內容 |
|---|---|
| `~/.local/share/art-term/openab-pty/` | binary、shell wrapper、設定、credential、manifest |
| `~/.config/systemd/user/art-term-openab-pty.service` | 使用者服務 |

先查看 `tailscale serve status --json`。**只有當 `8443` 唯一 handler 是 `/` → `http://127.0.0.1:18090`，且未新增其他 handler／Funnel，才可移除此 port**。這可能是原先就存在的相同路由；不確定所有權時請保留：

```bash
sudo tailscale serve --bg --https=8443 off
tailscale serve status
systemctl --user disable --now art-term-openab-pty.service
```

不要使用 `tailscale serve reset`。若路由已被其他服務修改，交管理者處理，不要刪整個 port。再確認 runtime 已停止，移除自己的安裝：

```bash
rm -- "$HOME/.config/systemd/user/art-term-openab-pty.service"
rm -r -- "$HOME/.local/share/art-term/openab-pty"
systemctl --user daemon-reload
```

### 明確升級既有 Host

升級要同時維持原認證與可恢復的安裝內容，因此不是直接覆寫執行中的 binary。腳本先準備完整 candidate（待切換安裝），記下 durable journal（持久交易紀錄），再停機交換；中斷時依紀錄辨識舊／新 bundle（包含 binary、設定與認證的完整目錄）。

```text
Old bundle running
       |
       v
Download + validate candidate -- failure --> old service untouched
       |
       v
Write durable intent --> stop --> atomic directory exchange
                                      |
                                      v
                                verify readiness
                                /              \
                         success                failure / interruption
                            |                         |
                     keep private backup       recover old bundle
                     write last record         verify original state
```

交換的單位是整個目錄，避免新 binary 與舊設定只更新一半；認證材料逐位元保留，App 不需重新登錄。但 session 是 Runtime 記憶體狀態，停機後無法復原。成功升級留下的 backup 也不等於已支援自動降版；下方復原指令只處理尚未完成的交易。

先保留 checkout 變更，再取得與交付相符的已審查更新。一般重跑不下載、不重啟、不旋轉 credential；`--upgrade-runtime` 將原腳本建立、通過 manifest／UID／權限／檔案核對的 format 1／2／3 安裝遷移到官方 format 4。已符合目前官方產物的安裝不重複替換。

```bash
bash scripts/init-host.sh --upgrade-runtime
# 或使用你信任且未修改的官方 binary：
bash scripts/init-host.sh --upgrade-runtime --binary /absolute/path/openab-pty --sha256 YOUR_64_HEX_SHA256
```

**`--upgrade-runtime` 本身就是結束所有既有 PTY 的明確授權。** 可非互動執行，不再詢問確認。所有 session、輸入狀態與 token 都會失去；完成後由使用者重新建立。credential、config、shell wrapper 與服務檔逐位元保留，App 原有認證不需重填。升級不能同時使用 `--serve`、`--enable-linger` 或檢視 flags。

腳本只接受 enabled、未 masked、無 drop-in、`Restart=on-failure` 的受管 user unit，且狀態為 active/running 或 inactive/dead。必須由正常登入取得有效 user bus、`XDG_RUNTIME_DIR` 與已啟用 linger。active 服務另核對 MainPID UID、執行檔 path/digest、unit path／載入內容與此 unit 的 cgroup；未知程序、failed／transitioning 狀態、殘留程序或不符設定會停止，不會代殺其他程序。原本 inactive 的服務升級後仍 inactive。

下載與 projection 驗證完成前不停止舊服務。完整 candidate 包含新 binary／manifest 與逐位元保留的 enrollment；先 fsync，再寫 durable intent journal。停機後使用 Linux `renameat2(RENAME_EXCHANGE)` 一次交換完整目錄，fsync parent。核心／檔案系統不支援交換時在停機前拒絕。服務 enabled 狀態與外部 unit 路徑不變。Tailscale 只讀取 `serve status --json` 的有效 Serve／Funnel 設定，比較前後 JSON；不修改、不復原路由。並行路由變更或無法查核時回報 preservation-unproven，不假稱入口保留成功。

### 中斷與復原

私人交易檔案都在 `$HOME/.local/share/art-term/`：`.upgrade-intent.json` 為 owner-only durable journal，`.upgrade-backup/` 為完整私人 bundle（含 credential），`.upgrade-last.json` 為最近完成／復原記錄。目錄 `0700`、檔案 `0600`（binary／shell `0700`）。請勿貼出或提交這些檔案。

若停電、SIGKILL 或下一次執行發現未完成 journal，一般操作會拒絕，請在原帳號執行：

```bash
bash scripts/init-host.sh --recover-upgrade
```

復原核對 journal 版本／路徑／UID、合法 old/new 完整 bundle 配對與目前服務 identity，必要時先 bounded stop，再 atomic exchange 回原 bundle，還原原 active/enabled 狀態並查核認證 readiness。catchable failure 也嘗試相同復原程序；不宣稱 SIGKILL／停電會執行 trap。session 無法復原。未知 PID、毀損／缺漏 backup 或 journal 會保留原狀與確切路徑，需人工查核，不能刪 journal 來繞過。

`--recover-upgrade` 只處理仍有 `.upgrade-intent.json` 的未完成交易。升級成功後只留下 `.upgrade-last.json` 與私人 backup，目前沒有已審查的自動回復舊版本流程；沒有 active journal 時，這個 flag 不會回復已完成的升級。若成功後仍需回到舊版本，先保留 backup 並安排明確的人工方案與審查，不要自行重建 journal 或把「保留備份」當作已支援自動 rollback。

readiness 的 bearer header 暫存檔放在 owner-only parent `$HOME/.local/share/art-term/.headers.*`，不放進受管 bundle。正常探測會刪除它；SIGKILL 留下的 `0600` 檔案仍含私人 credential，不能公開或提交，但不會令 bundle 檔案核對失敗。確認沒有 installer 執行後，才在私人目錄人工查核並移除已確認的暫存檔。

驗收成功且已確認不需要回復舊版本後，才由你明確移除最近的私人 backup；腳本不自動刪 credential 材料，也不累積多個備份。**先確認沒有 `.upgrade-intent.json` 且 `.upgrade-last.json` 是 VERIFIED／RECOVERED，再移除下列精確目錄：**

```bash
rm -r -- "$HOME/.local/share/art-term/.upgrade-backup"
```

若 backup 仍存在，下一次舊版升級會拒絕。相同官方安裝的檢視／no-op 不需刪 backup。最後記錄可保留；不要取消 linger，除非確認此帳號沒有其他 user service 依賴它。

路徑、UID、私有權限與 digest 在取得 installer flock 後及 commit 前重核對，檔案讀取使用 directory FD 和 O_NOFOLLOW。這保護正常安裝與協作衝突；擁有同一 UID 的惡意程序可同時改私人檔案，超出此個人帳號工具的隔離保證。Runtime session 是記憶體狀態，診斷交給 journald，不是 rollback 的持久資料。Host 升級 fixture 不取代 nuc、Windows GUI、Android 或 Hana 的實機驗收。

## instance-mcp 與桌面準備

openab-pty 提供 Shell 的 HTTPS／WSS；instance-mcp 提供桌面截圖與瀏覽器工具。兩者是獨立服務，token 不能互換。`init-host.sh` 只安裝 openab-pty，不會安裝 instance-mcp、桌面環境或瀏覽器。

已有共享 instance-mcp 的部署可讀取該服務的桌面截圖，瀏覽器也需使用同一個有畫面的桌面環境。單憑 openab-pty 網址與 token，不能宣稱桌面已配置完成。

在桌面服務帳號正常登入終端後，可檢查既有服務；以下 service 名稱僅為範例，需由管理者替換。以下命令只查狀態，不安裝或重啟：

```bash
systemctl --user is-active oab-instance-mcp.service oab-seat.service oab-pw-mcp.service
systemctl --user is-enabled oab-instance-mcp.service oab-seat.service oab-pw-mcp.service
```

這三個名稱不是通用部署契約，不可直接當成安裝命令。instance-mcp 與桌面環境由管理者依既有部署準備；本專案不代裝。App 可達的 MCP URL／token 在設定頁新增為獨立 Screen，截圖直接回 App，不經 Runtime PTY。需要 AI 工具時，受控端同帳號也須可達相同上游，再安裝[記憶體 stdio adapter](../refarch/runtime-desktop.md)，不保存上游設定。

純終端不需要 Screen，也不需重啟 Runtime 啟用桌面。舊 `--enable-desktop`／`--recover-desktop`／`--rollback-desktop` 只用於相符的舊安裝交易；新版不要求這條轉送路徑。已有交易先查紀錄及用途，不刪 journal 繞過檢查，不刪共用 instance-mcp 或其他 AI 工具配置。新版選擇、停止預覽與取消工具的差異見[桌面預覽](../refarch/desktop.md)。

## 附錄：實作與參考

[scripts/init-host.sh](../../scripts/init-host.sh)、[Host bundle](../../scripts/openab-runtime/host_bundle.py)、[官方產物](../../scripts/openab-runtime/official_artifact.py)。

安裝與管理入口：[Tailscale Linux 下載](https://tailscale.com/download/linux)、[Tailscale 手機／桌面下載](https://tailscale.com/download)、[Machines](https://login.tailscale.com/admin/machines)、[DNS 設定](https://login.tailscale.com/admin/dns)、[Serve 指南](https://tailscale.com/docs/features/tailscale-serve)、[存取控制指南](https://tailscale.com/docs/features/access-control)。上游背景：[openab-pty 建置與架構](https://github.com/openabdev/openab-pty#building)。
