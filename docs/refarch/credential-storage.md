# 憑證、執行時檔案與清理

連線檔案需要能重開使用，也需要避免複製檔案就取得管理憑證。App 因此把可直接讀取的連線 metadata 與保護後的密文一起存進唯一 TOML。Windows 使用 DPAPI，Android 使用 Keystore，Ubuntu 終端機版使用 Unix 帳號權限保護的本機金鑰。各客戶端的資料位置與金鑰保護方式分開，搬移使用加密匯出／匯入。

```text
connections.toml
+---------------------------------------+
| id / origin / revision / capabilities |
| credential_ciphertext                 |
+-------------------+-------------------+
                    |
       CredentialContext + platform unseal
                    |
       +------------+--------------------+
       |                                 |
Windows current-user DPAPI     Android operator Keystore key
                                         |
                              key-used-v1 marker

unsealed credential -> native memory -> Runtime HTTPS
session token        -> native memory -> WSS binding
Screen token         -> native memory -> direct MCP / memory-only adapter
```

CredentialContext 是憑證的驗證情境，將密文綁定 schema、id、origin、revision 與 App context；Android 的 AAD 是加密驗證時使用但不另行加密的附加資料。任意更改這些欄位或搬動密文，不能把它變成另一筆有效憑證。session token 是 Runtime 發給該次連線的短期授權，只在 native 記憶體，不持久化或經 IPC 讀回。

Android 的 marker 記錄 operator key 曾經使用過，並不保存金鑰。它讓 App 能辨識「全新安裝」與「曾有資料但金鑰遺失」，避免將新的、無法解開舊密文的 key 當成成功復原。資料、憑證、建置產物與 log 不放 docs 或提交 Git。

## 目前資料與保留的舊檔

| 位置 | 用途與保存 |
|---|---|
| Windows exe旁 connections.toml／--config | 唯一 schema 5／6 來源，含 Host／CLI 與獨立 Screens 的 DPAPI 密文；缺檔可正常空清單開啟，需可寫目錄 |
| Windows `<app_local_data_dir>/runtime-hosts-v1.json` | 0.1.17 舊資料，保留但不再讀取；升級前以舊 App 匯出後在新版匯入 |
| Android `<noBackupFilesDir>/openab-operator-v1/connections.toml` | 唯一 schema 5／6 來源，含 Host／Screen metadata 與 operator AES-GCM 密文 |
| Android 同目錄 `runtime-hosts-v1.json` | 舊資料保留但不讀取；不自動遷移 |
| 同目錄 key-used-v1 | key 已使用的持久 marker，刪全部 Host 仍保留；key 缺失且已有 marker／連線資料時拒絕建立替代 key |
| AndroidKeyStore alias art-term-openab-operator-wrap-v1 | AES-256-GCM wrapping key，AAD綁schema/id/origin/revision與app context；不在一般檔案或Git中 |
| Windows `%APPDATA%/art-term/known_hosts-<hash>` | 歷史 SSH pin，保留但 App 不再讀寫 |
| Android `<noBackupFilesDir>/ssh-identity-v1.bin` | 歷史 SSH identity 加密 blob，保留但 App 不再讀寫；Keystore alias `art-term-ssh-wrap-v1`，AAD `net.missart.artterm:ssh-identity:v1`，與 operator 分離 |
| 同目錄 `ssh-identity-used-v1`、`ssh-identity-v1.init`、`ssh-identity-v1.tmp`、`ssh-identity-v1.lock` | 歷史 SSH 保護狀態及鎖，App 不再讀寫，也不主動刪除 |
| Android `<app_data_dir>/host-keys.json`、`host-keys.lock` | 歷史 pin 資料，保留但目前 App 不再讀寫 |
| APK | 不含 connections.toml 預設 asset 或原生嵌入設定 |
| TUI `${XDG_CONFIG_HOME:-$HOME/.config}/art-term-tui/connections.toml` | 唯一 Host／Screen TOML，含 TUI 主金鑰保護的密文 |
| TUI TOML 同目錄 `<filename>.account-key-v1` | `0600` 的隨機帳號金鑰，直接啟動，無額外密碼包裝 |
| TUI TOML 同目錄 `<filename>.vault-v1` | 舊版密碼 metadata；安全空狀態升級時保留原檔作備份，不用於新啟動 |
| TUI TOML 同目錄 `<filename>.vault-lock` | 程序間互斥；儲存及其 cipher 的所有持有者離開後才釋放 |
| 使用者在系統選檔器指定的連線 JSON | 含管理憑證時以檔案密碼加密整份內容；不含憑證時為明文設定。自行保管／搬移，不放 Git 或公開文件 |

HostStore 使用同目錄暫存檔及原子替換，revision/context 保護憑證綁定；讀檔／解密失敗保留原資料。保存／取得 snapshot 前核對載入檔案的指紋；檔案被外部修改或刪除時拒絕過期操作，請重啟。Host revision 與匯入預覽的 store snapshot 核對是程序內 compare-and-swap（CAS，狀態仍符合讀取基線才套用）保護；HostStore 本身沒有跨程序 CAS／鎖或 watcher。Windows／Android 仍不能多個 App 同寫；TUI 另以 vault lock 拒絕第二個寫入者。備份前關閉所有使用該設定的程序。

## 純終端的帳號金鑰

TUI 不要求啟動或解鎖密碼，也不依賴桌面 keyring 或 D-Bus。首次安全初始化建立隨機 32-byte 帳號金鑰。`<filename>.account-key-v1` 是 7-byte `ARTTAK\x01` header 加上原始金鑰，共 39 bytes，沒有密碼包裝。Unix 帳號與檔案權限是保護邊界；同帳號程序能讀取金鑰。每筆憑證仍以 AES-256-GCM、全新 nonce 與完整 CredentialContext 加密，沿用 `ATTUI` 記錄格式。

設定目錄必須屬於目前使用者且為 `0700`，TOML、帳號金鑰與鎖檔為 `0600`。程式拒絕 symlink、hardlink、特殊檔案與不符格式或長度的金鑰。鎖檔必須為空檔，儲存及 cipher 的所有持有者離開後才釋放程序鎖。自訂 `--config` 的父目錄必須先存在且符合相同權限。

只有 TOML 不存在，且既有 `.vault-v1` 是正確的 84-byte `ARTTVLT\x01` metadata 時，才允許自動初始化新帳號金鑰；舊 metadata 原樣保留。已有 TOML 的舊密碼 vault 不會自動轉換，須另行安排遷移或使用原版加密匯出。已有設定卻缺少金鑰、格式損壞或憑證無法解密時，程式拒絕開啟並保留原資料，不提示啟動密碼或建立替代金鑰。

刪除最後一筆連線仍保留帳號金鑰。備份需保留整個設定目錄；跨裝置使用加密搬移檔，不直接複製 GUI 平台密文。搬移檔的檔案密碼及核心 Argon2id 加密仍保留，與 TUI 啟動無關。詳細操作見 [Ubuntu 終端機版](../platforms/linux.md)。

## Screen token 的保存與傳送

獨立 Screen 的 `token_ciphertext` 和 Host 存在同一 TOML，使用相同平台 CredentialCipher、獨立 domain／id／MCP URL／revision 驗證 context。修改同 URL 可保留 token，改 URL 必須重填。新增第一筆 Screen 前，若原檔存在，同目錄建立的私有備份 `connections.pre-desktops.toml` 也包含密文，不是跨裝置可解密的搬移檔；詳見[設定遷移](connections.md#舊設定與版本升級)。

App 原生層解密 token 後直接呼叫 MCP，不經 WebView 讀回。使用者明確接入工具時，透過關 echo 的短命 configure Shell 傳給受控端 adapter，只有行程記憶體保存本次 URL／token，不放 command line、環境變數、PTY 回顯或 log。adapter 的 owner-only socket 防其他帳號存取，同 UID 程序仍是信任邊界。

停止預覽不會清除 adapter 的工具憑證；「取消桌面工具」送 clear，關頁也 best-effort clear，斷網可能無法立即送達。stdin EOF／CLI 結束會釋放 adapter 記憶體。完整握手、大小限制與生命週期見[工具 bridge](runtime-desktop.md)。連線搬移可同時包含 Host 與 Screens；Screen token 僅隨整份密碼加密檔搬移，目的裝置再以自己的平台金鑰保護。未選取的本機 Screen 保留，分頁桌面選擇與工具授權不搬移。詳見[連線搬移](connection-transfer.md)。

## 金鑰遺失時為何拒絕開啟

Android 已有 operator key 時沿用它；初始化若發現 marker 缺少，會補寫 marker。只有 key 缺失，且 key-used marker、connections.toml、舊 runtime-hosts-v1.json 也全部不存在時，才允許建立全新 key。key 缺失但上述任一狀態存在就拒絕建立替代 key；只刪 marker 或 TOML 不能作為修復方式，刪除全部連線也保留 marker。

DPAPI復原需原Windows帳號／環境；Android的blob與Keystore須配對，單拷blob不是跨機備份。清 data／卸載會破壞 operator 與 SSH identity 連續性，不能當成修復部分遺失／損壞的方法。更新APK必須用原certificate保持app沙盒。Android cloud backup／device-transfer 已禁用；HTTP MCP 僅依原生受限 Tailnet／loopback 規則開放，不因此允許 WebView 任意 cleartext 外連。

跨裝置請使用[連線匯出／匯入](connection-transfer.md)，在新裝置匯入後重新以本機 DPAPI、Keystore 或 TUI 帳號金鑰保存。這條路徑不搬移上述平台金鑰或 session token；檔案密碼不持久化，解密預覽有兩分鐘期限。

目前不主動保存 scrollback、密碼或截圖到 log；終端／圖片在記憶體。測試與打包 log 由工具明示建立，分享前去除主機與終端內容。

## 建置與簽章

`target/`、`dist/`、node_modules、GUI gen/android及vault android/build/.tauri是本機生成物。target/android-tools/environment.json記錄JDK/SDK/NDK路徑；SDK/NDK可能體積很大，不與source同步。原生建置不複製或嵌入連線設定。

簽章預設 `%LOCALAPPDATA%/art-term/signing/`，包含 android-release.p12（alias art-term）、android-release.dpapi（原Windows帳號保護的密碼）與certificate-sha256.txt。key/DPAPI不得覆寫或提交；用原已驗certificate核對更新，不能以新產物自稱原簽章。build/sign/verify支援SigningRoot與ExpectedCertificate。

私有 bootstrap 已退役；建置拒絕啟用，不載入舊 seed／marker 作為連線來源。`config/connections.toml`與`config/*.private.toml` ignored，私人JSON與備份放repo外安全位置。

## 清理邊界

關閉使用中的程序後才按用途精確清理task-owned暫存／舊build，勿廣泛刪SDK、Owner產物或使用者temp。刪Host profile與關App都不刪遠端Shell；遠端狀態及runtime升級由 [Host 維護](../platforms/linux-host-maintenance.md) 說明。共享桌面／browser可能有私人畫面，不隨client關頁清理。
## 附錄：實作與參考

[crates/art-term-core/src/openab/profiles.rs](../../crates/art-term-core/src/openab/profiles.rs)、[crates/art-term-core/src/openab/secret.rs](../../crates/art-term-core/src/openab/secret.rs)、[crates/art-term-gui/src/platform/windows/dpapi.rs](../../crates/art-term-gui/src/platform/windows/dpapi.rs)、[crates/tauri-plugin-mobile-vault/android/src/main/java/net/missart/artterm/vault/MobileVaultPlugin.kt](../../crates/tauri-plugin-mobile-vault/android/src/main/java/net/missart/artterm/vault/MobileVaultPlugin.kt)。
