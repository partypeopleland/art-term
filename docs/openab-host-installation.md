# 在 Linux Host 原生安裝 openab-pty

本指南將官方 openab-pty Runtime、Shell 工具與 HTTPS 入口安裝在同一台 Linux Host。遠端 Shell 使用指定一般帳號的 HOME、工作目錄與軟體。若要操作 Host，Runtime 就必須在 Host 執行；在 Pod 裝 Runtime、在 Host 裝 Python，兩邊不會共用環境。

這份流程依 Ubuntu 24.04 x86_64、WSL2、systemd user service 與 Windows art-term 的實際安裝整理。已驗證 Host Shell、tmux 啟動、歷史控制、輸入確認與指令輸出，使用者也確認能輸入。未在其他平台重新安裝驗證；以下範例已做靜態核對，沒有將整份文件當腳本在乾淨主機重跑。

安裝要同時完成兩件事：

- 準備 Host 軟體與完整的 Shell 輔助程式。
- 確認執行帳號、HOME、檔案權限及 default ACL，讓輔助程式能接受新建狀態檔。

## 1. 確認安裝位置與帳號

在目標 Host 的一般使用者終端、完整 art-term checkout 根目錄操作。使用日後執行遠端 Shell 的帳號，不使用 `sudo bash` 啟動 Runtime 或工具 installer。

```bash
id -un
printf 'HOME=%s\n' "$HOME"
hostname
pwd
uname -m
systemctl --user show-environment >/dev/null
git rev-parse HEAD
git status --short
```

確認輸出對應預定 Host、帳號及 checkout。systemd user manager 必須可用。需要安裝系統套件時才使用 sudo；服務與輔助程式仍由一般帳號執行。

檢查既有 `art-term-openab-pty.service`、HTTPS listener 及工具目錄。存在安裝時先讀設定、保留 credential 與使用者資料，不覆寫目錄或清除所有 session 重來。重啟 Runtime 會中斷工作，先安排中斷時間。

## 2. 補齊 Host 軟體

| 用途 | 必備內容 |
|---|---|
| 基本工具 | Bash、Git、curl、Python 3、CA bundle、coreutils、util-linux、systemd／loginctl |
| 下載與權限檢查 | `sha256sum`、`tar`、`xz`、`getfacl`、`setfacl` |
| Shell 歷史 | 私有 Node 22、tmux **3.6a**、`tmux-shell.py`、自帶依賴的 `tmux-history.cjs` |
| Shell broker | `instance_bridge.py`、`instance-shell.py`、`instance-shell.bash` 與持久化 wrapper |
| 本指南的 LAN HTTPS 入口 | OpenSSL、socat |

Ubuntu 缺少套件時安裝：

```bash
sudo apt-get update
sudo apt-get install -y git curl python3 ca-certificates coreutils util-linux xz-utils acl openssl socat
```

本次確認的版本包括 Python 3.12.3、私有 Node 22.23.5、tmux 3.6a。系統原有 Node 24 與 tmux 3.4 可以保留；art-term 使用固定的私有路徑。

Runtime 使用未修改的官方 binary，不需要 Rust、Cargo 或 Docker。只有自行編譯 tmux 時才需要下列建置依賴：

```bash
sudo apt-get install -y gcc make pkg-config libevent-dev libncurses-dev bison
```

coding CLI、個人登入、instance-mcp 及桌面環境按需安裝。Shell broker 是新 Shell 的必要工具，不因未使用桌面而省略。

## 3. 安裝私有 Node、tmux 與所有輔助程式

由 Runtime 的同一個一般帳號準備下列路徑，目錄與執行檔均為 `0700`：

```text
$HOME/.local/share/art-term/
  openab-pty/
    node                       Node 22 執行檔
    tmux-v2/
      tmux                     tmux 3.6a 執行檔
      tmux-shell.py
      tmux-history.cjs
  tools/
    instance_bridge.py
    instance-shell.py
    instance-shell.bash
    tmux-shell.py
```

### 準備固定版本 binary

從 [Node 官方下載目錄](https://nodejs.org/dist/v22.23.5/)取得符合 Host 架構的 Node 22.23.5 archive，使用同目錄的 `SHASUMS256.txt` 核對後解壓，只將 `bin/node` 安裝到上表的 `openab-pty/node`。保留 LICENSE 與來源紀錄，不覆蓋系統 Node。

從 [tmux 官方 3.6a release](https://github.com/tmux/tmux/releases/tag/3.6a)取得 `tmux-3.6a.tar.gz`。本次驗證的 SHA256 為：

```text
b6d8d9c76585db8ef5fa00d4931902fa4b8cbe8166f528f44fc403961a3f3759
```

在獨立暫存目錄解壓後執行 `./configure`、`make`，將產生的 `tmux` 安裝到 `openab-pty/tmux-v2/tmux`。不替換系統 tmux，不連接個人既有 tmux server。保留來源授權與版本紀錄；安裝既有目的檔前先核對版本，不盲目覆寫。

### 建置並安裝歷史 helper

在可信且完整的 checkout 建置 helper。這一步需要符合專案要求的 Node 與 npm，可在建置機完成；目標 Host 不需要 `node_modules`。

```bash
npm --prefix apps/client ci
node scripts/openab-runtime/build-tmux-history.mjs --paged
```

將下列來源以 `0700` 安裝到表列位置：

- `scripts/openab-runtime/tmux-v2/tmux-shell.py`
- `target/tmux-history/tmux-v2/tmux-history.cjs`

接著在目標 Host、同一帳號、checkout 根目錄執行官方工具 installer：

```bash
bash scripts/install-instance-tools.sh --with-tmux-wrapper
```

這會安裝 `tools/` 下的 bridge、supervisor、Shell rc 與 wrapper。installer 不寫入 URL、token 或 CLI 全域設定，也不重啟現有 Shell。App 新建 Shell 優先選擇 `tools/tmux-shell.py`，缺少時才退回 `openab-pty/tmux-v2/tmux-shell.py`。安裝後建立新 Shell，既有工作不會自動改用新 supervisor。

確認命令與檔案存在：

```bash
python3 --version
"$HOME/.local/share/art-term/openab-pty/node" --version
"$HOME/.local/share/art-term/openab-pty/tmux-v2/tmux" -V
ldd "$HOME/.local/share/art-term/openab-pty/tmux-v2/tmux"
"$HOME/.local/share/art-term/openab-pty/node" --check "$HOME/.local/share/art-term/openab-pty/tmux-v2/tmux-history.cjs"
python3 - <<'PY_CHECK'
from pathlib import Path
base = Path.home() / '.local/share/art-term'
for name in ('instance_bridge.py', 'instance-shell.py', 'tmux-shell.py'):
    path = base / 'tools' / name
    compile(path.read_bytes(), str(path), 'exec')
compile((base / 'openab-pty/tmux-v2/tmux-shell.py').read_bytes(), 'tmux-shell.py', 'exec')
assert (base / 'tools/instance-shell.bash').is_file()
PY_CHECK
```

`ldd` 不得出現 `not found`。語法檢查通過只代表檔案可解析，仍需完成第 6 節的實際輸入驗證。

## 4. 在新建 Shell 前檢查權限與 default ACL

歷史狀態放在：

```text
$HOME/.local/share/art-term/tmux-history-v2/
  <24位十六進位Shell key>/
    manifest.json
    tmux.conf
    tmux.sock
```

專用 base 與每個工作目錄必須屬於 Runtime 帳號、權限為 `0700`。`manifest.json` 與 `tmux.conf` 必須為 `0600`。不要使用 symlink 導向其他帳號或共用目錄。

**`umask 0077` 不足以取代 ACL 檢查。** 本次 Host 的 HOME 有 default ACL，新狀態檔即使使用此 umask 仍產生 `0664`。Node helper 嚴格要求 `0600`，因此在 ready 前拒絕，client 將輸入維持為「送達尚未確認」，不自動重送。

以同一一般帳號建立或檢查專用 base。先確認既有路徑的 owner 正確且不是 symlink；以下命令只處理 art-term 專用目錄：

```bash
set -e
state="$HOME/.local/share/art-term/tmux-history-v2"
test ! -L "$state"
install -d -m 700 "$state"
test "$(stat -c %u "$state")" = "$(id -u)"
getfacl -p "$state"
setfacl -k -- "$state"
chmod 700 -- "$state"
getfacl -p "$state"
```

修復後最後一次 `getfacl` 輸出不應再有 `default:` 項。移除 base 的 default ACL 後，新建工作目錄與檔案不會再繼承這組 ACL。不要修改整個 HOME 的 ACL，不要執行全目錄 `chmod -R` 或放寬成 `777`。

若修復已存在的工作，從完整 `th2-<key>` session 名稱取出 24 位 key，核對目錄 owner 與檔案類型後，只處理該工作。不要對 symlink 執行修復。例如將下列 `SHELL_KEY` 換成實際 key：

```bash
SHELL_KEY=填入24位十六進位key
[[ "$SHELL_KEY" =~ ^[0-9a-f]{24}$ ]]
session_state="$state/$SHELL_KEY"
test ! -L "$session_state"
test "$(stat -c %u "$session_state")" = "$(id -u)"
setfacl -k -- "$session_state"
chmod 700 -- "$session_state"
test -f "$session_state/manifest.json" && test ! -L "$session_state/manifest.json"
test -f "$session_state/tmux.conf" && test ! -L "$session_state/tmux.conf"
chmod 600 -- "$session_state/manifest.json" "$session_state/tmux.conf"
stat -c '%a %U %n' "$state" "$session_state" "$session_state/manifest.json" "$session_state/tmux.conf"
```

修復後仍須新建測試 Shell，確認它的狀態檔**自然產生為 `600`**。不能把每次手動 chmod 當成完成安裝。補裝 broker 後也需新建 Shell，才能套用新的 supervisor。

## 5. 啟動 Host 原生 Runtime 與 HTTPS 入口

先選定入口方式：

- **LAN／IP 直連**使用本節的 Host Runtime 與 Host TLS proxy。Tailscale 不屬於這條路徑的必要軟體。
- **Tailscale** 可使用既有 `scripts/init-host.sh --enable-linger --serve` 與可信 `*.ts.net` HTTPS 名稱。該腳本目前即使不加 `--serve` 也會檢查 Tailscale，不可將它當成免 Tailscale 的 LAN installer。既有安裝與升級見 [Host 維護](platforms/linux-host-maintenance.md)。

兩條路徑都在 Host 執行官方 binary，Runtime 只監聽 loopback，由真正的 HTTPS 入口轉送。以下 LAN 範例使用 `127.0.0.1:18090`。

### LAN 首次安裝範例

以下為本次已驗證拓樸的設定範本，僅用於**尚未安裝的 Host**。先選擇服務目錄，已有內容或同名 user service 時停止，先規劃保留原 credential 的遷移；不直接覆寫。

```bash
set -e
runtime="$HOME/.local/share/art-term/openab-pty-native"
test ! -e "$runtime"
test ! -e "$HOME/.config/systemd/user/art-term-openab-pty.service"
install -d -m 700 "$runtime"
```

從 checkout 的固定官方下載器取得原生 binary。它從 GHCR 讀取並驗證固定 image、config、layer 與 binary 雜湊，只抽出執行檔，**不啟動容器**：

```bash
PYTHONPATH=scripts/openab-runtime python3 - "$runtime" <<'PY_RUNTIME'
import json, os, re, subprocess, sys
from pathlib import Path
import official_artifact
root = Path(sys.argv[1])
os.umask(0o077)
binary = root / 'openab-pty'
with binary.open('xb') as output:
    output.write(official_artifact.fetch())
binary.chmod(0o700)
(root / 'source.json').write_text(json.dumps(official_artifact.provenance(), indent=2) + '\n')
result = subprocess.run([str(binary), '--generate-admin-credential'], check=True, capture_output=True, text=True)
credential = re.search(r'^credential: ([a-f0-9]{64})$', result.stdout, re.M).group(1)
verifier = re.search(r'^admin_credential_hash = "(sha256:[a-f0-9]{64})"$', result.stdout, re.M).group(1)
(root / 'credential').write_text(credential + '\n')
(root / 'credential').chmod(0o600)
(root / 'shell').write_text('#!/bin/bash\nexec /bin/bash -i\n')
(root / 'shell').chmod(0o700)
config = f'''[pty]
enabled = true
listen = "127.0.0.1:18090"
tls_terminated_upstream = true
command = {json.dumps(str(root / 'shell'))}
max_sessions = 8
absolute_session_ttl = "12h"
attach_token_ttl = "15m"
detached_idle_ttl = "8h"
scrollback_kib = 1024
scrollback_replay = false
admin_credential_hash = "{verifier}"
'''
(root / 'config.toml').write_text(config)
(root / 'config.toml').chmod(0o600)
subprocess.run([str(binary), '--validate-projection', str(root / 'config.toml')], check=True)
PY_RUNTIME
```

`tls_terminated_upstream = true` 只適用於接下來確實建立的 TLS 入口，不表示可把 HTTP 裸露到 LAN。管理 credential 保存在 `0600` 檔案，由擁有者在自己的互動終端讀取並填入 App；不要貼到聊天、issue、日誌或版本庫。既有安裝不要重產 credential。

準備 `server.crt`、`server.key`，放入 `$runtime`，private key 為 `0600`。正常驗證時，憑證須由 client 信任的 CA 簽發、在有效期內且符合網址；IP 網址須有對應的 **IP address SAN**，只填 CN 不足以通過檢查。client 使用內建 CA 清單，匯入 Windows 或瀏覽器的自建 CA 不會自動被 art-term 信任。

Windows／Android 正式版可在 **設定 → 連線設定 → 開發模式：略過伺服器憑證驗證** 勾選，供自簽或 LAN IP 開發測試。此設定預設關閉並會保存，影響後續 Runtime HTTPS 管理請求與新建或重新 attach 的 WSS 連線；切換後重新連線即可套用，不需特別重做開發版。開啟時略過 CA、名稱與有效期檢查，因此不必讓 client 匯入自建 CA，也不必先取得公開 CA 憑證或符合網址的 SAN。入口仍須提供 TLS 憑證與私鑰，傳輸仍使用 HTTPS／WSS 加密與 token 授權，但不驗證伺服器身分，僅供開發使用。此開關不影響 Screen MCP，Ubuntu 終端機版目前仍使用正常驗證。完整範圍與保存方式見[開發模式設定](refarch/connections.md#runtime-開發模式)。

建立下列兩個 user service。`%h` 與 `%u` 由 systemd 展開。這些範例使用上方固定的 `openab-pty-native` 目錄；若變更路徑，同步修改 unit 與設定。

`~/.config/systemd/user/art-term-openab-pty.service`：

```ini
[Unit]
Description=art-term native Host Runtime
[Service]
ExecStart=%h/.local/share/art-term/openab-pty-native/openab-pty --config %h/.local/share/art-term/openab-pty-native/config.toml
WorkingDirectory=%h
Environment=HOME=%h USER=%u SHELL=/bin/bash TERM=xterm-256color LANG=C.UTF-8
Environment=PATH=%h/.local/bin:%h/.cargo/bin:/usr/local/bin:/usr/bin:/bin
UMask=0077
NoNewPrivileges=true
LimitCORE=0
Restart=on-failure
RestartSec=3
KillMode=control-group
TimeoutStopSec=15
[Install]
WantedBy=default.target
```

將真實 LAN IPv4 填入 `$runtime/ingress.env`，不要原樣使用範例值。先確認該 IP 屬於 Host，且埠未被其他服務占用：

```text
OPENAB_HOST_IP=192.0.2.10
OPENAB_HTTPS_PORT=8443
```

`~/.config/systemd/user/art-term-openab-pty-tls.service`：

```ini
[Unit]
Description=art-term native LAN TLS ingress
After=art-term-openab-pty.service
Requires=art-term-openab-pty.service
[Service]
EnvironmentFile=%h/.local/share/art-term/openab-pty-native/ingress.env
ExecStart=/usr/bin/socat OPENSSL-LISTEN:${OPENAB_HTTPS_PORT},bind=${OPENAB_HOST_IP},reuseaddr,fork,cert=%h/.local/share/art-term/openab-pty-native/server.crt,key=%h/.local/share/art-term/openab-pty-native/server.key,verify=0 TCP:127.0.0.1:18090
UMask=0077
NoNewPrivileges=true
LimitCORE=0
Restart=on-failure
RestartSec=3
KillMode=control-group
TimeoutStopSec=15
[Install]
WantedBy=default.target
```

socat 的 `verify=0` 表示入口不要求 TLS client certificate；Runtime 仍使用 admin credential／session token 驗證，與 art-term 是否驗證伺服器憑證是不同設定。

```bash
chmod 600 "$runtime/server.key" "$runtime/ingress.env"
systemctl --user daemon-reload
systemctl --user enable --now art-term-openab-pty.service art-term-openab-pty-tls.service
sudo loginctl enable-linger "$(id -un)"
systemctl --user is-active art-term-openab-pty.service art-term-openab-pty-tls.service
loginctl show-user "$(id -un)" -p Linger
ss -lntp 'sport = :8443'
```

WSL2 mirrored network 的這次驗證使用 **Host 上實際監聽 LAN IP 的程序**。Pod 的 `hostPort`、Pod 內健康回應，或刻意指定 CNI source 才成功的請求，不能證明另一台筆電能連到 Host。不要為排查此服務改寫全機路由或其他服務。

## 6. 驗證入口、Shell 與輸入

1. 確認 user services 為 `active`／`enabled`、linger 為 `yes`，並核對 listener 的程序確實在 Host。用 `journalctl --user -u art-term-openab-pty.service -n 50` 檢查啟動錯誤，不輸出 credential。
2. 從使用端的正常網路路徑連到實際 HTTPS URL。未帶 token 的 `/admin/sessions` 應回 `401`；若自建 CA，檢查工具需指定 CA。不要只測 `127.0.0.1`，也不要把 HTTP `200` 當成 Shell 已可用。
3. 在 art-term 填 URL 與 admin credential，建立**全新 Shell**。確認提示符、`hostname`、`id -un`、`printf '%s\n' "$HOME"`、`pwd` 都是預定 Host 與帳號，再確認 `python3 --version`。
4. 輸入 `printf 'HOST_INPUT_OK\n'` 並按 Enter，確認畫面出現完整標記。向上捲動進歷史，再輸入另一條無副作用指令，確認能返回即時畫面並執行。不能接受「輸入送達尚未確認」或重送命令才偶爾成功。
5. 用該 Shell 的完整 `th2-<key>` 檢查專用狀態目錄，確認 `manifest.json` 與 `tmux.conf` 為 `600`、工作目錄為 `700`，新工作不需手動修權限。
6. 確認左下方沒有 Shell broker 查詢錯誤。沒有選 Screen 時，上游桌面 target 為空是正常的，不代表必須安裝桌面套件。

若做協定層檢查，使用與 client 相同的 Node history worker，經 Runtime 啟動並驗證 `ready`、含輸入的 `follow` 確認回覆與原終端輸出；broker 控制查詢需成功。此 worker 會驗證 Runtime／父程序身分，直接在 SSH 終端手動執行不能代替該驗證。原始 WSS 能寫入只證明 PTY 收發，不能取代歷史控制及 broker 的檢查。

測試只使用自己建立的工作，完成後只清理該工作。安裝、升級或排查不需要無條件刪除使用者全部 session。若維護者無法操作 Windows／Android App，回報「Host 驗證通過，待使用者 App 實測」，不要宣稱已完成使用端驗收。

## 排查常見錯誤

| 現象 | 先檢查 |
|---|---|
| `python3: not found` | 提示符是否仍是容器帳號；在真正執行 Runtime 的環境核對 Python。Host 安裝工具不會補進 Pod。 |
| 有提示符但「輸入未送出／送達尚未確認」 | 檢查歷史 helper 是否 ready，以及專用 manifest 的 owner、`600` 權限與 default ACL。不要盲目重送可能已執行的輸入。 |
| helper 關閉 `4004` | 此次實例是 manifest `0664` 被拒絕；核對權限、檔案型別與對應 Runtime 身分，不把所有 `4004` 都視為同一原因。 |
| 「無法恢復 Shell 的既有 Screen／Shell broker 查詢」 | 確認四個 `tools/` 檔完整，以相同帳號安裝。建立新 Shell 載入 supervisor；不必先安裝桌面。 |
| HTTPS 憑證驗證失敗 | 正常模式核對 IP SAN／DNS、有效期與內建 CA 信任鏈；Windows／Android 自簽測試可啟用開發模式後重新連線。只匯入作業系統 CA 不會改變 art-term 的信任清單。 |
| localhost 可連、LAN 逾時 | 核對 Host listener、正常來源路徑與網路政策，尤其是 WSL。HTTP 入口與 PTY／輸入能力分別驗證。 |
| 更新 helper 後仍顯示舊錯誤 | 保留工作資料，關閉舊分頁並建立新 Shell；不要為套用工具刪除全部工作。 |

## 參考

- [使用前準備](openab-host-setup.md)
- [終端歷史與主機工具](refarch/terminal-history.md#主機準備)
- [Shell 工具 bridge](refarch/runtime-desktop.md#安裝與既有連線)
- [Tailscale installer 與既有 Host 維護](platforms/linux-host-maintenance.md)
- [官方 Runtime 固定產物下載器](../scripts/openab-runtime/official_artifact.py)
- [Shell 工具 installer](../scripts/install-instance-tools.sh)
- [openab-pty 上游](https://github.com/openabdev/openab-pty)
