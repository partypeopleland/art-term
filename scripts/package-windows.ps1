param(
    [switch]$SkipBuild,
    [string]$ConfigPath = "$PSScriptRoot/../config/connections.example.toml",
    [string]$OutputPath = "$PSScriptRoot/../dist/windows/art-term-0.1.19"
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
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
        Push-Location (Join-Path $projectRoot 'apps/desktop')
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
        '需要 Windows 10 1809 以上、WebView2 Runtime 與可達 Runtime Host 的網路。',
        '開啟 art-term-gui.exe，按「新增 openab-pty 連線」選擇 Host／Agent 分類，填入可信 HTTPS 根網址與管理憑證。',
        '點名稱開啟 Shell；右側「Shell」可列出、選擇或建立工作。', '',
        '程式旁 connections.toml 是唯一連線來源；不存在、空白或 connections = [] 時顯示空清單，可直接新增或匯入。',
        '新增、編輯、刪除及匯入皆寫回同一份 TOML（schema_version=5）；管理憑證以目前 Windows 使用者 DPAPI 保護後保存，沒有明文憑證。',
        '換裝置：在「管理」多選匯出連線，含憑證時用密碼加密；自行搬移檔案後，在新裝置選檔解鎖並勾選匯入。',
        '相同網址預設略過，明確勾選才更新；不含憑證的 JSON 在新裝置需逐筆補入管理憑證。',
        '修改設定後重啟；覆寫路徑：art-term-gui.exe --config C:/path/connections.toml。',
        '不讀取使用者目錄中的舊 runtime-hosts-v1.json；舊版本連線需先在舊 App 匯出，再於新版匯入。',
        '請將整包放在可寫目錄；手動修改或刪除 TOML 前先關閉 App。刪除後重開為空清單，舊檔與金鑰不會自動移除。',
        'Android 同樣使用裝置內唯一 connections.toml；首次使用可新增或匯入，APK 不嵌入預設連線。',
        '桌面沿用既有管理憑證，由 App 建立專用截圖 Shell 呼叫 Host helper，圖片經原 Runtime 連線回傳；不會把截圖指令送進工作 Shell。',
        '整個交付資料夾一起搬移，請保留設定與授權檔案。',
        '關分頁只本機脫離；遠端 Shell 仍受 TTL 及 runtime 生命週期限制。'
    )
    Set-Content -LiteralPath (Join-Path $outputRoot 'README.md') -Value $instructions -Encoding UTF8
    $productVersion = (Get-Content -LiteralPath (Join-Path $projectRoot 'crates/art-term-gui/tauri.conf.json') -Raw | ConvertFrom-Json).version
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
