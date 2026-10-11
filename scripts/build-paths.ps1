# Rust panic/source locations can expose the builder's local directories even
# when the product configuration is public. Remap them for distributed builds.
function Get-ArtTermBuildRustFlags {
    param([Parameter(Mandatory)][string]$ProjectRoot, [string]$AndroidTarget)
    $flags = @()
    if ($env:CARGO_ENCODED_RUSTFLAGS) {
        $flags += $env:CARGO_ENCODED_RUSTFLAGS.Split([char]31)
    } elseif ($env:RUSTFLAGS) {
        # Cargo also splits RUSTFLAGS on whitespace; encoded flags preserve spaces.
        $flags += $env:RUSTFLAGS.Trim() -split '\s+'
    }
    foreach ($prefix in @($env:USERPROFILE, $ProjectRoot)) {
        if (-not $prefix) { continue }
        $replacement = if ($prefix -eq $ProjectRoot) { '/art-term' } else { '/build-home' }
        foreach ($spelling in @($prefix, $prefix.Replace('\', '/')) | Select-Object -Unique) {
            $flags += "--remap-path-prefix=$spelling=$replacement"
        }
    }
    if ($AndroidTarget -eq 'aarch64') {
        $flags += '-C link-arg=-Wl,-z,max-page-size=16384' -split ' '
    }
    return [string]::Join([char]31, [string[]]$flags)
}
