#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
output_dir="$project_root/dist/linux"
while (($#)); do
    case "$1" in
        --output-dir)
            [[ $# -ge 2 && -n "$2" ]] || { echo '--output-dir requires a path' >&2; exit 2; }
            output_dir=$2
            shift 2
            ;;
        *) echo 'Usage: scripts/package-linux.sh [--output-dir PATH]' >&2; exit 2 ;;
    esac
done
[[ $(uname -s) == Linux && $(uname -m) == x86_64 ]] || {
    echo 'Build this package on Ubuntu 24.04 amd64.' >&2
    exit 1
}
for tool in cargo npm node python3 dpkg-deb; do
    command -v "$tool" >/dev/null || { echo "Missing build tool: $tool" >&2; exit 1; }
done
mkdir -p -- "$output_dir"
output_dir=$(cd -- "$output_dir" && pwd)
[[ -z $(find "$output_dir" -mindepth 1 -maxdepth 1 -print -quit) ]] || {
    echo 'Output directory must be new or empty; preserve delivered packages.' >&2
    exit 1
}
cd -- "$project_root/apps/client"
npm ci --no-audit --no-fund
npm run build
cd -- "$project_root/crates/art-term-gui"
node ../../apps/client/node_modules/@tauri-apps/cli/tauri.js build --ci --bundles deb -- --locked
cd -- "$project_root"
target_dir=$(cargo metadata --no-deps --format-version 1 --locked | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
python3 - "$project_root" "$target_dir" "$output_dir" <<'PY'
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

root, target, output = map(Path, sys.argv[1:])
version = json.loads((root / "crates/art-term-gui/tauri.conf.json").read_text())["version"]
package = target / "release/bundle/deb" / f"art-term_{version}_amd64.deb"
if not package.is_file():
    raise SystemExit(f"Expected freshly built package not found: {package}")
fields = subprocess.check_output(
    ["dpkg-deb", "--field", str(package), "Package", "Version", "Architecture"], text=True
)
if fields.splitlines() != ["Package: art-term", f"Version: {version}", "Architecture: amd64"]:
    raise SystemExit(f"Unexpected package identity: {fields}")
shutil.copy2(package, output / package.name)
for source, name in [(root / "LICENSE", "LICENSE"), (root / "docs/third-party-notices.md", "THIRD-PARTY-NOTICES.md")]:
    shutil.copy2(source, output / name)
(output / "README.md").write_text(
    "# art-term Ubuntu\n\n"
    "Ubuntu 24.04 amd64，需圖形桌面及已解鎖的 Secret Service 金鑰圈。\n"
    f"安裝：sudo apt install ./{package.name}\n"
    "從應用程式選單開啟 art-term，或執行 art-term-gui。\n"
    "預設設定為 ${XDG_CONFIG_HOME:-$HOME/.config}/net.missart.artterm.desktop/connections.toml。\n"
    "指定其他可寫路徑：art-term-gui --config /path/connections.toml。\n"
    "憑證由目前使用者的 Secret Service 金鑰保護。換裝置請使用 App 的加密匯出／匯入。\n"
    "勿刪除金鑰圈項目或 operator-key-used-v1 標記；遺失金鑰後不能解密原設定。\n"
    "移除套件不會清除使用者設定與金鑰。此套件不包含受控端 Runtime。\n",
    encoding="utf-8",
)
manifest = {"platform": "ubuntu-24.04-amd64", "version": version, "files": []}
for path in sorted(output.iterdir()):
    if path.is_file():
        manifest["files"].append({"file": path.name, "bytes": path.stat().st_size, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
(output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
(output / "SHA256SUMS").write_text("".join(f'{entry["sha256"]}  {entry["file"]}\n' for entry in manifest["files"]))
print(f"Ubuntu package: {output / package.name}")
PY
