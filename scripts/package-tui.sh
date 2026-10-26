#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
output_dir="$project_root/dist/tui"
while (($#)); do
    case "$1" in
        --output-dir)
            [[ $# -ge 2 && -n "$2" ]] || { echo '--output-dir requires a path' >&2; exit 2; }
            output_dir=$2
            shift 2
            ;;
        *) echo 'Usage: scripts/package-tui.sh [--output-dir PATH]' >&2; exit 2 ;;
    esac
done
[[ $(uname -s) == Linux && $(uname -m) == x86_64 ]] || {
    echo 'Build this candidate on Ubuntu x86_64.' >&2
    exit 1
}
mkdir -p -- "$output_dir"
output_dir=$(cd -- "$output_dir" && pwd)
[[ -z $(find "$output_dir" -mindepth 1 -maxdepth 1 -print -quit) ]] || {
    echo 'Output directory must be new or empty.' >&2
    exit 1
}
cd -- "$project_root"
cargo build -p art-term-tui --bin art-term --release --locked
target_dir=$(cargo metadata --no-deps --format-version 1 --locked | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
python3 - "$project_root" "$target_dir" "$output_dir" <<'PY'
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib

root, target, output = map(Path, sys.argv[1:])
version = tomllib.loads((root / "crates/art-term-tui/Cargo.toml").read_text())["package"]["version"]
name = f"art-term-{version}-linux-x86_64"
binary = target / "release/art-term"
subprocess.run([str(binary), "--version"], check=True)
with tempfile.TemporaryDirectory(prefix="art-term-package-") as directory:
    bundle = Path(directory) / name
    bundle.mkdir()
    shutil.copy2(binary, bundle / "art-term")
    shutil.copy2(root / "LICENSE", bundle / "LICENSE")
    shutil.copy2(root / "docs/third-party-notices.md", bundle / "THIRD-PARTY-NOTICES.md")
    shutil.copytree(root / "crates/art-term-tui/licenses", bundle / "licenses")
    (bundle / "README.md").write_text(
        "# art-term Ubuntu 終端機版\n\n"
        "候選目標 Ubuntu 24.04 x86_64，可在 Server、Desktop 或 SSH 終端執行 ./art-term，不需要圖形桌面。\n"
        "可將 art-term 複製至自己的 ~/.local/bin，勿以 sudo 啟動。\n"
        "首次啟動設定解鎖密碼，之後每次輸入相同密碼。密碼無法重設，請保留加密匯出備份。\n"
        "預設設定在 ${XDG_CONFIG_HOME:-$HOME/.config}/art-term-tui；請保留整個目錄與 vault metadata。\n"
        "使用 --config /path/connections.toml 可指定其他檔案，相關金鑰與鎖定資料與它同目錄。\n"
        "使用 --help 查看選項，畫面內的 Help 查看快捷鍵。主機仍使用既有 art-term Runtime/helper。\n"
        "關閉本機分頁只會脫離，不刪除遠端 Shell。本版不提供圖形桌面預覽。\n",
        encoding="utf-8",
    )
    archive = output / (name + ".tar.gz")
    with tarfile.open(archive, "w:gz") as tar:
        tar.add(bundle, arcname=name)
metadata = {
    "version": version,
    "platform": "ubuntu-x86_64-terminal",
    "archive": archive.name,
    "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
    "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
}
(output / "manifest.json").write_text(json.dumps(metadata, indent=2) + "\n")
(output / "SHA256SUMS").write_text("".join(
    hashlib.sha256(p.read_bytes()).hexdigest() + "  " + p.name + "\n"
    for p in sorted(output.iterdir()) if p.is_file() and p.name != "SHA256SUMS"
))
print(f"Terminal package: {archive}")
PY
