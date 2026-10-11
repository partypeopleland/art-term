# AI 操作指南：安裝官方 openab-pty 受控端

本頁供使用者的 AI 執行 Linux Host 安裝。目標是讓 art-term 以 HTTPS／WSS 操作指定帳號的遠端 Shell；使用者只需準備依賴與在 App 填入連線資訊。給使用者看的入口是[使用前準備](openab-host-setup.md)，進階故障、升級與復原細節見 [Host 維護](platforms/linux-host-maintenance.md)。

## 執行範圍與完成條件

依使用者指定的主機、帳號與授權操作；本頁不是更動其他主機或既有系統服務的授權。指令預設在 **Linux 受控端的專案根目錄** 執行，不是 Windows／Android 使用端。先確認目前所在主機與專案路徑；無法操作目標時，回報缺少的連線或權限，不把本機安裝當成遠端已完成。

完成時應具備：未修改的官方 Runtime、指定帳號的 systemd 使用者服務、可達的私人 HTTPS 入口、保留於受保護位置的管理憑證，以及 art-term 的實際 Shell 驗證。若最後一步需使用者操作 App，明確列為待使用者確認，不以安裝成功取代。

## 1. 檢查依賴與既有狀態

先讀取專案的協作指引，核對 checkout 版本與未提交內容。沿用已有專案目錄，不覆蓋私人檔案、不另外複製工作環境。若尚無專案或必要腳本，取得使用者指定的交付版本；不能只複製單一 shell 腳本。

| 檢查 | 要求與處理 |
|---|---|
| 作業系統 | 預設流程為 Ubuntu 22.04／24.04 x86_64，且具備 Bash、systemd 使用者服務。ARM64 需提供可信、相符的官方 binary 與來源 SHA256，依 Host 維護指南的指定 binary 路徑處理。其他平台不可套用此腳本。 |
| 帳號 | 以日後操作 Shell 的一般使用者正常登入；不可用 root 或 `sudo bash` 執行 installer。必須有有效的 systemd user manager。需要系統設定時由該帳號使用 sudo。 |
| 基本工具 | Git、curl、Python 3、CA 憑證，以及系統的 coreutils、util-linux、systemd／loginctl。腳本會檢查 timeout、flock、sha256sum、install 等指令。Ubuntu 缺少套件時，依授權使用套件管理器補齊。 |
| 網路 | Host 與使用端已登入正確 Tailnet；Host 有可用的 `*.ts.net` 名稱、MagicDNS 與 HTTPS certificates。存取政策需允許使用端到 Host TCP 8443。缺少 Tailscale 時依官網處理，不在本文件另維護其安裝流程；不自行重設 Tailnet 政策或現有路由。 |
| 下載 | 新裝預設需能以 HTTPS 取得官方 GHCR 產物。受限網路可使用 Host 維護指南的可信 binary 方式，不跳過來源與 SHA256 驗證。 |
| coding CLI | 只在使用者需要時處理，需安裝及登入於相同 Shell 帳號。Runtime 不代裝 CLI。 |
| 桌面 | 只有使用者要求桌面預覽才需要 instance-mcp、可用桌面環境與 helper 設定；不阻擋純終端安裝。 |

**不要把 Rust、Cargo、rustup、Docker 或 App 的編譯工具鏈加入受控端前置。** 預設 installer 用 Python 從固定官方 GHCR 映像抽取 binary，驗證 image／config／layer／binary 雜湊後原生執行，沒有容器或原始碼編譯步驟。也不要卸載既有 Rust／Cargo，其他工作可能仍依賴它們。

完整 checkout 應包含 `scripts/init-host.sh` 與 `scripts/openab-runtime/` 的 host_bundle.py、official_artifact.py、desktop_bundle.py、preview.py、mcp_preview.py。保留腳本 LF 換行。

可用以下唯讀查核確認目標環境；不要輸出任何 credential 檔案內容：

```bash
pwd
id -un
uname -m
systemctl --user show-environment >/dev/null
tailscale status
tailscale serve status
bash scripts/init-host.sh --help
```

檢查是否已有 `~/.local/share/art-term/openab-pty/` 與 `~/.config/systemd/user/art-term-openab-pty.service`。已有其中之一時視為既有或未完成安裝，先查核，不刪目錄重新開始。存在升級／桌面交易紀錄時，依維護指南處理復原。

## 2. 選擇首次安裝或既有安裝

### 首次安裝

前置具備且使用者授權此主機安裝後，以一般帳號在專案根目錄執行：

```bash
bash scripts/init-host.sh --enable-linger --serve
```

`--enable-linger` 讓此帳號的服務在開機與登出後持續執行；`--serve` 新增私人 HTTPS 8443 入口。這兩項可能要求使用者輸入 sudo 密碼或完成 Tailscale HTTPS 同意。AI 無法代填時，直接說明使用者需要在何處完成，保留腳本回報與已完成狀態。

腳本會檢查前置、下載並驗證官方 binary、建立管理憑證、啟用 `art-term-openab-pty.service`、驗證認證，最後設定 Serve。Runtime 僅監聽 `127.0.0.1:18090`，Shell 原生使用安裝帳號的 HOME／檔案／CLI。不得為了方便改成容器工作環境、公開 Funnel 或開放 loopback port。

若 Runtime 已裝好但 Serve 未完成，先處理腳本指出的 HTTPS 同意或網路缺口，再執行：

```bash
bash scripts/init-host.sh --serve
```

8443 已被其他路由使用時保留原服務並回報衝突；不得執行 `tailscale serve reset` 或移除其他路由。

### 已有安裝

先使用檢視入口：

```bash
bash scripts/init-host.sh --show-info
```

一般重跑沿用現有 credential，不會自動下載新 Runtime 或重啟服務。不要用 fresh install 覆蓋既有目錄，也不要重新產生金鑰來處理驗證失敗。

需要從舊版遷移時，先讀 [Host 維護的升級與復原](platforms/linux-host-maintenance.md#明確升級既有-host)，確認既有工作中斷影響與已取得的授權，再使用 `--upgrade-runtime`。此操作會終止全部既有 PTY；備份可保留安裝內容與憑證，不能恢復執行中的 Shell。未取得中斷授權時停在查核／方案階段。復原 flags 只用於相符交易，不刪 journal 繞過檢查。

## 3. 驗證並交付連線資訊

確認 installer 正常結束、user service active／enabled、登入者 linger 已開啟，以及 Serve 8443 指向本機 Runtime。腳本會驗證正確認證可讀取 session 清單、錯誤認證收到 401；這仍不代表使用端實際連線已通過。

```bash
systemctl --user is-active art-term-openab-pty.service
systemctl --user is-enabled art-term-openab-pty.service
loginctl show-user "$(id -un)" -p Linger
tailscale serve status
bash scripts/init-host.sh --show-info
```

交給使用者的網址須沿用輸出的 `Base URL`，形式如 `https://workstation.example-tailnet.ts.net:8443`。此處為虛構例子；實際交付填入查得的名稱，不以 IP 取代 DNS，也不降低 TLS 驗證。

管理憑證由使用者在自己的互動終端執行以下命令取得，直接填入 App 或自己的密碼管理器。**AI 不執行並收集此輸出，不把憑證帶進聊天、日誌或版本庫。**

```bash
bash scripts/init-host.sh --show-credential
```

請使用者在 art-term 新增 openab-pty 連線，填入名稱、Host 分類、Base URL 與管理憑證，測試連線後建立 Shell。使用 `pwd` 或其他無副作用操作確認是指定主機／帳號；需要 CLI 時再確認其可啟動。沒有實際操作 App 的能力，就將這一步交給使用者，不宣稱已完成手機或 Windows 驗收。

## 4. 只有需要時才啟用桌面預覽

使用者提出桌面需求後，先依 [Runtime 桌面 helper](refarch/runtime-desktop.md) 確認可達的 instance-mcp 與桌面環境，準備 owner-only MCP 設定，再使用 `--enable-desktop`。不得把 token 寫進命令列或對話。這只設定本專案 helper，不修改官方 openab-pty，也不代裝 instance-mcp 或接管其他共享服務。

## 回報格式

回覆使用者時只交付實際主機／帳號、安裝或沿用結果、App 連線網址、安全取得憑證的方法、驗證結果與必要的待辦。區分「服務已就緒」與「App 已實測」。不貼 token、完整設定、冗長執行日誌或與當次安裝無關的內部細節。

## 附錄：實作與官方參考

[初始化入口](../scripts/init-host.sh)、[安裝與交易處理](../scripts/openab-runtime/host_bundle.py)、[官方產物下載器](../scripts/openab-runtime/official_artifact.py)、[Tailscale 下載](https://tailscale.com/download)、[Serve 官方文件](https://tailscale.com/docs/features/tailscale-serve)。固定官方 Runtime 來源與雜湊由下載器管理；本指南不指示 AI 追蹤最新浮動版本或自行改寫來源。
