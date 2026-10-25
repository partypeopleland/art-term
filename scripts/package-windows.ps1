param(
    [switch]$SkipBuild,
    [string]$ConfigPath = "$PSScriptRoot/../config/connections.example.toml",
    [string]$OutputPath
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$productVersion = (Get-Content -LiteralPath (Join-Path $projectRoot 'crates/art-term-gui/tauri.conf.json') -Raw | ConvertFrom-Json).version
if (-not $OutputPath) { $OutputPath = Join-Path $projectRoot "dist/windows/art-term-$productVersion" }
$outputRoot = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputPath)
$ConfigPath = (Resolve-Path -LiteralPath $ConfigPath).Path
if ((Test-Path -LiteralPath $outputRoot) -and (Get-ChildItem -LiteralPath $outputRoot -Force | Measure-Object).Count -gt 0) {
    throw 'OutputPath must be new or empty; preserve previously delivered packages.'
}
. "$PSScriptRoot/build-paths.ps1"
$previousRustFlags = $env:CARGO_ENCODED_RUSTFLAGS
$previousConfig = $env:ART_TERM_CONNECTIONS_FILE
$env:CARGO_ENCODED_RUSTFLAGS = Get-ArtTermBuildRustFlags -ProjectRoot $projectRoot
$env:ART_TERM_CONNECTIONS_FILE = $ConfigPath
try {
    if (-not $SkipBuild) {
        Push-Location (Join-Path $projectRoot 'apps/client')
        try {
            & npm.cmd ci
            if ($LASTEXITCODE -ne 0) { throw 'npm ci failed' }
            & npm.cmd run build
            if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed' }
        } finally { Pop-Location }
        Push-Location $projectRoot
        try {
            & cargo build -p art-term-gui --release --locked
            if ($LASTEXITCODE -ne 0) { throw 'GUI build failed' }
        } finally { Pop-Location }
    }
    New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
    Copy-Item -LiteralPath (Join-Path $projectRoot 'target/release/art-term-gui.exe') -Destination (Join-Path $outputRoot 'art-term-gui.exe')
    Copy-Item -LiteralPath $ConfigPath -Destination (Join-Path $outputRoot 'connections.toml')
    Copy-Item -LiteralPath (Join-Path $projectRoot 'LICENSE') -Destination (Join-Path $outputRoot 'LICENSE')
    Copy-Item -LiteralPath (Join-Path $projectRoot 'docs/third-party-notices.md') -Destination (Join-Path $outputRoot 'THIRD-PARTY-NOTICES.md')
    $instructions = @(
        '# art-term Windows', '',
        '需要 Windows 10 1809 以上、WebView2 Runtime 與可達 Runtime Host／instance-mcp 的網路。',
        '開啟 art-term-gui.exe；固定頂列的品牌開啟版本與更新紀錄，同頁左欄預設200px，可拖曳分隔線調寬，左下預覽可拖曳上緣調高並等比例縮放，右側為分頁與終端。',
        '齒輪開啟全頁設定，可選科技感、深黑、柔光或懷舊主題，並新增 Host／Agent／Screen、匯入或匯出 Host。',
        '在主機旁點既有工作名稱 attach／接管，＋建立新的歷史工作；多筆可展開。齒輪設定以左側分類、右側內容呈現，連線直接編輯、刪除與測試，不另開管理燈箱。', '',
        '程式旁 connections.toml 是唯一設定來源；不存在、空白或 connections = [] 時正常顯示空清單，不嵌入私人連線。',
        'Host 與 Screens 分開保存於同一 TOML；沒有 Screen 為 schema 5，有 Screen 為 schema 6。管理憑證及 Screen token 由目前 Windows 使用者 DPAPI 保護，不保存明文。',
        '新增第一筆 Screen 前，既有 TOML 備份為同目錄 connections.pre-desktops.toml；降版前先確認相容及備份，不靜默丟棄桌面。',
        '換裝置：在齒輪的連線設定選擇匯出 Host／Agent、Screens 或混合清單；管理憑證與 Screen token 以密碼加密整份檔案。自行搬移後在新裝置解鎖、勾選匯入，未選取的本機設定保留。',
        '相同網址預設略過，明確勾選才更新；不含憑證的 JSON 在新裝置需逐筆補入管理憑證。',
        '手動改設定前先關閉 App；覆寫路徑：art-term-gui.exe --config C:/path/connections.toml。請放在可寫目錄。',
        '不讀取使用者目錄中的舊 runtime-hosts-v1.json；舊版本連線需先在舊 App 匯出，再於新版匯入。',
        '每個 live Shell 可獨立選 Screen；App 原生層直接 MCP 取圖，不把截圖命令輸入工作 Shell。停止預覽與取消桌面工具分開操作。',
        '需要 Shell／coding CLI 工具時，先在受控端安裝通用 broker helper，再建立新連線；不注入正在執行的 CLI。Screen 選擇限定該 Shell，不是 Host 全域設定。',
        '即時輸出與歷史都使用原本的終端視窗；上捲閱讀、下捲到底自然接續即時輸出。受控端須使用支援scroll／follow的通用helper；Android既有快照介面維持相容，沒有Windows專用helper。',
        '終端中的 HTTP／HTTPS 網址點擊後由系統預設瀏覽器開啟，歷史內容也能使用；滑動與拖曳選取不開啟網址。',
        '整個交付資料夾一起搬移，請保留設定、備份與授權檔案；DPAPI 密文不能直接移給不同 Windows 帳號。',
        '關分頁只本機脫離；遠端工作仍受 TTL／Runtime 生命週期限制，退出工作 Bash 則結束該工作。'
    )
    Set-Content -LiteralPath (Join-Path $outputRoot 'README.md') -Value $instructions -Encoding UTF8
    $manifest = foreach ($name in @('art-term-gui.exe','connections.toml','README.md','LICENSE','THIRD-PARTY-NOTICES.md')) {
        $path = Join-Path $outputRoot $name
        [ordered]@{file=$name;sha256=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant();bytes=(Get-Item -LiteralPath $path).Length}
    }
    [ordered]@{platform='windows-x64';version=$productVersion;configuration_sha256=(Get-FileHash -LiteralPath (Join-Path $outputRoot 'connections.toml')).Hash.ToLowerInvariant();build=if($SkipBuild){'existing binary; verify source separately'}else{'cargo build -p art-term-gui --release --locked'};files=@($manifest)} |
        ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $outputRoot 'manifest.json') -Encoding UTF8
    Write-Output "Windows package: $outputRoot"
} finally {
    $env:ART_TERM_CONNECTIONS_FILE = $previousConfig
    $env:CARGO_ENCODED_RUSTFLAGS = $previousRustFlags
}
