# 連線匯出、匯入與新裝置還原

Windows 與 Android 共用連線匯出／匯入流程；Android 入口位於全站齒輪設定頁的「連線設定」，Windows 保留「管理 openab-pty 連線」入口。檔案由使用者自行搬移；App 不提供帳號、雲端備份或自動同步。正常公開建置也能還原，不需要私人 bootstrap、來源裝置的 DPAPI／Keystore 金鑰或特殊 APK。

0.1.18 不再讀取 0.1.17 的使用者目錄 Host JSON。升級前先用舊版完成匯出，再開啟新版匯入；舊資料與平台金鑰保留。不要以複製 DPAPI／Keystore 密文取代跨裝置匯出。

## 憑證如何跨過裝置邊界

```text
Source device                     User-carried file             Destination
connections.toml                                               connections.toml
      |                                                              ^
DPAPI / Keystore unseal                                               |
      |                                                       local seal + save
      v                                                              |
settings + credential -> password-encrypted JSON -> unlock -> preview / select
                              |
                    salt + nonce + ciphertext
```

來源裝置先用自己的平台金鑰解開已保存的管理憑證，再把設定與憑證一起放入密碼保護檔。目的裝置使用檔案密碼解鎖，預覽並確認項目後，以自己的 DPAPI／Keystore 重新保護（seal）並保存。搬移的是可還原的設定與憑證，不是平台金鑰、Shell 分頁或 session binding；因此新裝置不必取得來源裝置的金鑰，也不會接管來源工作。

## 匯出／匯入生命週期

匯出由選擇清單選擇 1–64 筆連線，明確決定是否包含管理憑證。包含時要求檔案密碼及確認；使用原生檔案選擇器保存後，由使用者自行搬移。密碼不持久化，遺失時只能由原裝置重新匯出。

匯入依序為選檔、解鎖、預覽、勾選、一次套用。預覽顯示名稱／網址／分類；新連線含憑證時預設勾選，相同 origin 預設略過，僅明確選擇才更新本機。若本機有多筆同 origin，該項只能略過，需先整理。完成後回報新增／更新數；驗收需測試連線、Shell、已宣告的桌面，以及重開 App 後持久性。

Windows 使用原生檔案視窗，Android 使用系統文件選擇器。新裝置仍須自行準備可達 Runtime 的網路／Tailnet 與可信 HTTPS。不含憑證的匯入項目要明確勾選並補入管理憑證；更新已有 origin 時可保留本機憑證。

匯入不啟動 coding CLI，也不恢復來源裝置的 Shell 分頁或接管 session。CLI 名稱、命令及參數會隨能力設定保存，使用者仍須在已連線的 Shell 明確按下啟動按鈕。未在來源設定桌面／CLI 的連線不會因匯入而新增這些能力。

取消勾選「包含管理憑證」會產生一般 JSON，名稱、網址和能力均為明文。新裝置匯入這種檔案時，須先勾選項目並逐筆填入管理憑證；相同網址明確更新時，保留該裝置既有憑證。沒有管理憑證的手寫 TOML 連線只能做不含憑證的匯出；選擇含憑證會拒絕。這不是完整的憑證備份。

## 為什麼預覽後仍可能需要重來

預覽代表解鎖當下的匯入候選，不是對本機清單的永久授權。使用者選定項目後，原生層在同一 HostStore 鎖內再次核對完整 store 狀態；所有項目驗證與 seal 成功才保存一次。這是程序內 compare-and-swap（CAS）：只在目前狀態仍符合預覽基線時套用，避免以舊預覽覆蓋後來的修改。任一項失敗都不留下部分匯入。

預覽有兩分鐘期限、限定發起視窗且只能提交一次。取消、逾時或切到背景會清除待用密碼與解密預覽；已進入原子保存的提交則不因取消撤銷，返回管理後以實際清單為準。

## 檔案與平台保存契約

- 格式標記 `art-term-connections`、version `1`；匯出的是當下有效的名稱、canonical HTTPS origin、分類、Native 桌面／CLI metadata，以及選擇包含的管理憑證。metadata 直接取自唯一 connections.toml；還原後名稱、能力與重新保護的憑證均原子寫回此檔案。
- 含憑證時，名稱、網址、metadata 與憑證整份加密。固定 Argon2id v19、64 MiB、3 iterations、4 lanes 衍生 256-bit key，AES-256-GCM 驗證加密；每次使用 OS 隨機 16-byte salt、12-byte nonce。外層只保留格式、版本、固定演算法參數、salt、nonce 與密文；拒絕自行提高 KDF 成本的檔案。
- 密碼為 1–1024 UTF-8 bytes，原樣處理、不裁切空白。外層最多 2 MiB，明文 payload 最多 1 MiB，1–64 筆連線；同檔重複 origin、未知欄位或版本、非 canonical base64、損壞內容一律拒絕。一般 JSON 即使含 `credential: null` 也拒絕。
- 新增連線建立本機新 id；更新沿用本機 id 並遞增 revision。Windows 以目前使用者 DPAPI、Android 以既有 operator Keystore 重新保護憑證。來源 id/revision、平台金鑰、session/token/binding 均不匯出，既有平台金鑰不重建。
- 同時只執行一項檔案操作。取消、逾時或背景切換會清除待用的密碼及解密預覽；系統 picker 期間只允許非秘密待選狀態。提交已進入原子保存時，取消不會撤銷已提交內容，返回管理後以實際清單為準。

## 直接相關失敗處理

錯密碼與損壞密文共用提示，重新選檔後再輸入。預覽逾時、切到背景或連線設定已變更時，也需重新選檔預覽。不要從日誌、截圖或命令列分享密碼與管理憑證。

Windows 匯出先在同目錄寫暫存、同步後原子替換。Android 直接寫入系統選定的文件 URI；提供者寫入失敗時可能留下不完整檔案，App 會回報失敗，使用者應刪除該檔並重新匯出。App 不擅自刪除無法確認所有權的 URI，完成寫入也不代表雲端提供者已同步到其他裝置。

檔案成功匯入但不能連線時，先檢查新裝置的網路／Tailnet、Runtime HTTPS 憑證與服務，再確認管理憑證仍有效。匯入不代裝 Runtime、CLI 或桌面服務；初始化見 [Host 指南](../openab-host-setup.md)。平台建置與實機驗收步驟見 [Windows](../platforms/windows.md) 與 [Android](../platforms/android.md)。
## 附錄：實作與參考

[crates/art-term-core/src/openab/transfer.rs](../../crates/art-term-core/src/openab/transfer.rs)、[crates/art-term-core/src/openab/profiles.rs](../../crates/art-term-core/src/openab/profiles.rs)、[crates/art-term-gui/src/runtime_transfer.rs](../../crates/art-term-gui/src/runtime_transfer.rs)、[apps/desktop/src/components/RuntimeTransfer.vue](../../apps/desktop/src/components/RuntimeTransfer.vue)。
