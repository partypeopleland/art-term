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
    shutil.copy2(root / "crates/art-term-tui/keybindings.default.toml", bundle / "keybindings.example.toml")
    shutil.copy2(root / "crates/art-term-tui/display.default.toml", bundle / "display.example.toml")
    shutil.copy2(root / "docs/third-party-notices.md", bundle / "THIRD-PARTY-NOTICES.md")
    shutil.copytree(root / "crates/art-term-tui/licenses", bundle / "licenses")
    (bundle / "README.md").write_text(
        "# art-term Ubuntu 終端機版\n\n"
        "候選目標 Ubuntu 24.04 x86_64，可在 Server、Desktop 或 SSH 終端執行 ./art-term，不需要圖形桌面。\n"
        "可將 art-term 複製至自己的 ~/.local/bin，勿以 sudo 啟動。\n"
        "登入目前 Linux 帳號後即可直接使用，不需另外設定或輸入解鎖密碼。\n"
        "預設設定在 ${XDG_CONFIG_HOME:-$HOME/.config}/art-term-tui；請保留整個目錄，尤其是 connections.toml.account-key-v1 帳號金鑰；請勿公開或分開搬移。\n"
        "使用 --config /path/connections.toml 可指定其他檔案，相關金鑰與鎖定資料與它同目錄。\n"
        "使用 --help 查看選項，畫面內的 Help 查看快捷鍵。主機仍使用既有 art-term Runtime/helper。\n"
        "左上固定展開 Computer／Agent／Screen 樹，方向鍵與 Enter 或滑鼠操作；分頁可滑鼠切換／關閉。\n"
        "點右上 Desktop 選擇此 Shell 的桌面並配置工具；Tools ready 才代表 broker 已確認。支援的本機終端可顯示原生截圖。\n"
        "點左下縮圖在右側展開；拖曳平移、滾輪縮放、[Fit] 還原、[X]／Esc／再點縮圖收回。這些操作只控制本機檢視。\n"
        "display.toml 預設 auto，只用原生 Sixel，不用字格拼圖。SSH／tmux／screen／Linux 純文字主控台只停用預覽；桌面工具與 AI 呼叫照常可用。編碼器已內建。\n"
        "本機 auto 目前辨識 XTerm390（-ti vt340 -tn vt340 -b 0 -fn fixed）；其他已確認支援的本機終端可設定 sixel，仍須提供實際字格像素。缺少條件或圖片設定錯誤只停用預覽，連線仍可使用。\n"
        "F1 說明、F2 連線、F3 終端、F4 桌面、F10 Actions。編輯設定同目錄的 keybindings.toml 後重新啟動，可自訂快捷鍵。\n"
        "關閉本機分頁只會脫離，不刪除遠端 Shell。可攜式加密匯入／匯出仍使用檔案密碼。\n",
        encoding="utf-8",
    )
    for path in [bundle, *bundle.rglob("*")]:
        path.chmod(0o755 if path.is_dir() else 0o644)
    (bundle / "art-term").chmod(0o755)
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
