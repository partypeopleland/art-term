#!/usr/bin/env python3
"""Drive catalog, transfer, mouse and editable-keybinding features through the real TUI."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import tomllib

from run import McpFixture, Runtime, Terminal


class CatalogTerminal(Terminal):
    def modal_region(self):
        width = min(self.screen.columns - 4, 88)
        height = min(self.screen.lines - 4, 24)
        left = (self.screen.columns - width) // 2
        top = (self.screen.lines - height) // 2
        return left + 1, top + 1, left + width - 1, top + height - 1

    def modal_click(self, label):
        self.wait(lambda: bool(self.locations(label, self.modal_region())), "modal mouse target " + label)
        self.click(*self.locations(label, self.modal_region())[-1])
        for _ in range(4):
            self.pump()

    def action(self, label):
        self.click_text("[Actions]", (0, 0, self.screen.columns, 1))
        self.wait_text("Actions / ")
        region = self.modal_region()
        for _ in range(30):
            hits = self.locations(label, region)
            if hits:
                self.click(*hits[-1])
                for _ in range(4):
                    self.pump()
                return
            self.wheel((region[0] + region[2]) // 2, (region[1] + region[3]) // 2, 1)
        raise AssertionError("Mouse action is inaccessible: " + label)

    def fill(self, label, value, replace=False):
        self.modal_click(label)
        if replace:
            self.send(b"\x15")
        self.send(value)

    def settle(self, seconds=0.12):
        until = time.monotonic() + seconds
        while time.monotonic() < until:
            self.pump(min(0.03, max(0, until - time.monotonic())))

    def selected_tree(self):
        for row in range(2, self.screen.lines - 3):
            text = "".join(self.screen.buffer[row][column].data for column in range(min(40, self.screen.columns)))
            if text.lstrip("│ ").startswith("▶"):
                return text
        return ""

    def select_tree(self, label):
        self.click_text("[Connections]", (0, 0, self.screen.columns, 1))
        self.idle()
        self.settle()
        for _ in range(100):
            before = self.selected_tree()
            self.send(b"\x1b[A")
            self.settle()
            if self.selected_tree() == before:
                break
        for _ in range(100):
            if label in self.selected_tree():
                return
            self.send(b"\x1b[B")
            self.settle()
        raise AssertionError("Tree selection is inaccessible: " + label)

    def quit(self):
        self.action("Quit")
        self.finish()
        assert b"\x1b[?1006l" in self.raw, "Exit did not disable mouse reporting"


def config_path(home):
    return home / "config/art-term-tui/connections.toml"


def raw_config(home):
    path = config_path(home)
    return path.read_bytes() if path.exists() else None


def config(home):
    raw = raw_config(home)
    return tomllib.loads(raw.decode()) if raw is not None else {"connections": [], "desktops": []}


def wait_counts(app, home, hosts, screens):
    app.wait(lambda: len(config(home).get("connections", [])) == hosts
             and len(config(home).get("desktops", [])) == screens, "saved catalog counts")
    app.idle()


def import_form(app, path, password):
    app.action("Import")
    app.wait_text("File path")
    app.fill("File path", str(path))
    app.fill("File password", password)
    app.modal_click("[Save]")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=False)
    os.chmod(output, 0o700)
    checks = []
    clients = []
    runtime = None
    screen = None

    def passed(case, behavior):
        checks.append({"case": case, "result": "PASS", "behavior": behavior})
        print("PASS " + case + " " + behavior, flush=True)

    def start(home):
        app = CatalogTerminal(binary, home, runtime.ca)
        clients.append(app)
        app.start()
        app.idle()
        return app

    with tempfile.TemporaryDirectory(prefix="art-term-tui-catalog-") as directory:
        root = Path(directory)
        source = root / "source"
        target = root / "target"
        fixture = root / "fixture"
        for path in (source, target, fixture):
            path.mkdir(mode=0o700)
        runtime = Runtime(fixture)
        screen = McpFixture("catalog")
        transfer = root / "mixed-transfer.enc"
        file_password = "synthetic-transfer-password-Only"
        try:
            app = start(source)
            for label in ("Computer", "Agent", "Screen", "New Host", "New Screen"):
                app.wait_text(label)
            app.click_text("New Host", (0, 1, 40, 24))
            app.wait_text("HTTPS Runtime URL")
            app.fill("Name", "Catalog Host")
            app.fill("HTTPS Runtime URL", runtime.origin)
            app.fill("Credential", runtime.credential)
            app.modal_click("[Save]")
            wait_counts(app, source, 1, 0)
            assert runtime.credential.encode() not in raw_config(source)
            assert runtime.credential.encode() not in app.raw
            app.select_tree("Catalog Host")
            app.action("Test")
            app.wait_text("主機連線正常")
            passed("T-CRUD", "Mouse create/test Host and masked encrypted credential persistence")

            app.select_tree("Catalog Host")
            app.action("Edit")
            app.wait_text("編輯 Host")
            app.fill("Name", "Catalog Renamed", replace=True)
            app.modal_click("[Save]")
            app.wait(lambda: config(source)["connections"][0]["name"] == "Catalog Renamed", "Host edit saved")
            app.idle()
            app.select_tree("Catalog Renamed")
            app.action("Test")
            app.wait_text("主機連線正常")
            before = raw_config(source)
            app.select_tree("Catalog Renamed")
            app.action("Delete")
            app.wait_text("確認 / Confirm")
            app.modal_click("[Cancel]")
            assert raw_config(source) == before
            passed("T-CRUD", "Blank-credential edit keeps authentication; mouse delete Cancel preserves bytes")

            app.action("New Screen")
            app.wait_text("HTTPS MCP URL")
            app.fill("Name", "Catalog Screen")
            app.fill("HTTPS MCP URL", screen.url)
            app.fill("Token", screen.token)
            app.modal_click("[Save]")
            wait_counts(app, source, 1, 1)
            assert screen.token.encode() not in raw_config(source)
            assert screen.token.encode() not in app.raw
            app.action("Export")
            app.wait_text("Confirm password")
            app.fill("File path", str(transfer))
            app.fill("File password", file_password)
            app.fill("Confirm password", file_password)
            app.modal_click("[Save]")
            app.wait(lambda: transfer.exists(), "encrypted mixed export")
            app.wait_text("已加密匯出")
            for secret in (runtime.credential, screen.token, file_password):
                assert secret.encode() not in transfer.read_bytes()
                assert secret.encode() not in app.raw
            assert transfer.stat().st_mode & 0o777 == 0o600
            source_key = (source / "config/art-term-tui/connections.toml.account-key-v1").read_bytes()
            app.quit()
            passed("T-TRANSFER", "Actual TUI exports mixed Host/Screen through a masked encrypted file-password form")

            app = start(source)
            app.wait_text("Catalog Renamed")
            assert (source / "config/art-term-tui/connections.toml.account-key-v1").read_bytes() == source_key
            app.select_tree("Catalog Renamed")
            app.action("Test")
            app.wait_text("主機連線正常")
            app.quit()
            passed("T-CRUD", "Prompt-free restart preserves edited Host and account key and reauthenticates")

            restored = start(target)
            before = raw_config(target)
            import_form(restored, transfer, "incorrect-synthetic-password")
            restored.wait(lambda: "Authenticationrejected" in "".join(restored.text().split()), "wrong-password rejection")
            assert raw_config(target) == before
            import_form(restored, transfer, file_password)
            restored.wait_text("Import preview")
            restored.modal_click("[Cancel]")
            assert raw_config(target) == before
            import_form(restored, transfer, file_password)
            restored.wait_text("Import preview")
            restored.modal_click("[Apply]")
            wait_counts(restored, target, 1, 1)
            restored.wait_text("Catalog Renamed")
            restored.select_tree("Catalog Renamed")
            restored.action("Test")
            restored.wait_text("主機連線正常")
            assert runtime.credential.encode() not in raw_config(target)
            assert screen.token.encode() not in raw_config(target)
            passed("T-TRANSFER", "Wrong password and preview Cancel preserve state; mixed import restores real authentication")

            before = raw_config(target)
            import_form(restored, transfer, file_password)
            restored.wait_text("Import preview")
            restored.wait_text("[Replace duplicates: OFF]")
            restored.modal_click("[Apply]")
            restored.idle()
            assert raw_config(target) == before, "Default duplicate skip changed saved profiles"
            restored.select_tree("Catalog Renamed")
            restored.action("Edit")
            restored.wait_text("編輯 Host")
            restored.fill("Name", "Local Override", replace=True)
            restored.modal_click("[Save]")
            restored.wait(lambda: config(target)["connections"][0]["name"] == "Local Override", "local edit before duplicate replace")
            restored.idle()
            import_form(restored, transfer, file_password)
            restored.wait_text("Import preview")
            restored.modal_click("[Replace duplicates: OFF]")
            restored.wait_text("[Replace duplicates: ON]")
            restored.modal_click("[Apply]")
            restored.wait(lambda: config(target)["connections"][0]["name"] == "Catalog Renamed", "explicit duplicate replacement")
            wait_counts(restored, target, 1, 1)
            passed("T-TRANSFER", "Duplicate import skips by default and explicit Replace restores source profile without duplicates")

            restored.click_text("New Shell", (0, 1, 40, 24))
            restored.wait(lambda: len(runtime.work()) == 1, "mouse first work creation")
            first = next(iter(runtime.work()))
            restored.wait(lambda: runtime.work()[first]["bootstrap"], "first work bootstrap")
            restored.wait_text("Live")
            restored.action("New Shell")
            restored.wait(lambda: len(runtime.work()) == 2, "mouse second work creation")
            second = next(name for name in runtime.work() if name != first)
            restored.wait(lambda: runtime.work()[second]["bootstrap"], "second work bootstrap")
            restored.wait_text("2:")
            restored.wait_text("Live")
            restored.click_text("1:", (40, 1, 120, 2))
            restored.send("MOUSE-FIRST")
            restored.wait(lambda: "MOUSE-FIRST" in runtime.work()[first]["input"], "mouse first-tab input")
            restored.click_text("2:", (40, 1, 120, 2))
            restored.send("MOUSE-SECOND")
            restored.wait(lambda: "MOUSE-SECOND" in runtime.work()[second]["input"], "mouse second-tab input")
            assert "MOUSE-FIRST" not in runtime.work()[second]["input"]
            assert "MOUSE-SECOND" not in runtime.work()[first]["input"]
            frames = list(runtime.frames)
            restored.click_text("[Connections]", (0, 0, 120, 1))
            restored.send("LOCAL-TREE-INPUT")
            for _ in range(5):
                restored.pump()
            assert runtime.frames == frames
            restored.action("New Host")
            restored.wait_text("HTTPS Runtime URL")
            restored.click(100, 30)
            restored.send("MODAL-LOCAL-INPUT")
            restored.modal_click("[Cancel]")
            assert runtime.frames == frames
            assert len(config(target)["connections"]) == 1
            runtime.append_lines(second, 0, 60)
            restored.wheel(70, 10, -1)
            restored.wait(lambda: any(event.get("method") == "history" and event.get("action") == "scroll" and event.get("name") == second for event in runtime.snapshot()["events"]), "hovered terminal mouse history wheel")
            restored.action("Follow")
            restored.wait(lambda: any(event.get("method") == "history" and event.get("action") == "follow" and event.get("name") == second for event in runtime.snapshot()["events"]), "mouse Follow action")
            assert all(b"\x1b[<" not in data for _, data in runtime.frames), "Local SGR mouse reports reached work input"
            restored.click_text("[Help]", (0, 0, 120, 1))
            restored.wait_text("keybindings.toml")
            before_help = restored.text()
            restored.wheel(60, 10, 2)
            restored.wait(lambda: restored.text() != before_help, "mouse Help scroll")
            restored.click_text("[Close]")
            restored.wait(lambda: "Help / " not in restored.text(), "mouse Help close")
            restored.click_text("[x]", (40, 1, 80, 2))
            restored.wait(lambda: not runtime.work()[first]["attached"], "mouse local tab close")
            assert first in runtime.work() and second in runtime.work()
            assert not any(event["method"] == "DELETE" and event["name"] == first for event in runtime.snapshot()["events"])
            passed("T-MOUSE", "Real mouse tabs route input independently; tree/modal input stays local; terminal/Help wheels and tab-close preserve remote work")

            restored.resize(14, 20)
            restored.wait_text("[Actions]")
            restored.wait_text("[Desktop]")
            restored.action("New Host")
            restored.wait_text("[Save]")
            restored.wait_text("[Cancel]")
            restored.modal_click("[Cancel]")
            restored.wait(lambda: not restored.locations("[Save]"), "20-column mouse form Cancel")
            restored.resize(36, 120)
            restored.wait_text("[Connections]")
            restored.quit()
            passed("T-MOUSE", "Twenty-column window retains pointer access to Actions/Desktop and form Cancel")

            bindings = target / "config/art-term-tui/keybindings.toml"
            bindings.write_text('leader = "Ctrl+g"\n[bindings]\nhelp = ["F9"]\nhosts = ["Alt+h"]\nquit = ["leader+x"]\n')
            custom = start(target)
            custom.wait_text("Catalog Renamed")
            custom.send(b"\x1bOP")
            for _ in range(5):
                custom.pump()
            assert "keybindings.toml 自訂" not in custom.text(), "Disabled F1 still opens Help"
            custom.send(b"\x1b[20~")
            custom.wait_text("keybindings.toml")
            custom.wait_text("Ctrl+g")
            custom.wait_text("F9")
            custom.click_text("[Close]")
            custom.wait(lambda: "Help / " not in custom.text(), "custom-key Help close")
            custom.send(b"\x1dc")
            for _ in range(5):
                custom.pump()
            assert "Screen：選擇此 Shell" not in custom.text(), "Previous leader still selects Desktop"
            custom.send(b"\x07c")
            custom.wait_text("Stop")
            custom.modal_click("[Cancel]")
            before_work = set(runtime.work())
            custom.action("New Shell")
            custom.wait(lambda: len(runtime.work()) == len(before_work) + 1, "custom-key test work creation")
            custom_work = next(name for name in runtime.work() if name not in before_work)
            custom.wait(lambda: runtime.work()[custom_work]["bootstrap"], "custom-key test work bootstrap")
            custom.wait_text("Live")
            custom.send("CUSTOM-REMOTE-BASELINE")
            custom.wait(lambda: "CUSTOM-REMOTE-BASELINE" in runtime.work()[custom_work]["input"], "custom-key work input baseline")
            custom.click_text("[Terminal]", (0, 0, 120, 1))
            custom.send(b"\x1bh")
            for _ in range(5):
                custom.pump()
            frames = list(runtime.frames)
            custom.send("CUSTOM-LOCAL-NAV")
            for _ in range(5):
                custom.pump()
            assert runtime.frames == frames, "Custom direct Connections binding did not focus local tree"
            custom.send(b"\x07x")
            custom.finish()
            passed("T-KEYS", "Edited file changes leader/direct Help and Connections keys, disables old Help/leader and changes Quit after restart")

            invalid = '[bindings]\nhelp = ["leader+h"]\n'
            bindings.write_text(invalid)
            refused = CatalogTerminal(binary, target, runtime.ca)
            clients.append(refused)
            refused.finish(expected=1)
            assert bindings.read_text() == invalid
            assert "keybindings.toml" in refused.text()
            assert "duplicate shortcut" in refused.text()
            passed("T-KEYS", "Invalid duplicate bindings fail visibly with file/cause and preserve original bytes")

            app = start(source)
            app.select_tree("Catalog Renamed")
            app.action("Delete")
            app.wait_text("確認 / Confirm")
            app.modal_click("[Yes]")
            wait_counts(app, source, 0, 1)
            app.quit()
            app = start(source)
            wait_counts(app, source, 0, 1)
            assert not app.locations("Catalog Renamed", (0, 1, 40, 24))
            app.quit()
            passed("T-CRUD", "Confirmed Host deletion survives restart and preserves independent Screen")
            app = start(source)
            app.action("New Agent")
            app.wait_text("HTTPS Runtime URL")
            app.fill("Name", "Catalog Agent")
            app.fill("HTTPS Runtime URL", runtime.origin)
            app.fill("Credential", runtime.credential)
            app.modal_click("[Save]")
            wait_counts(app, source, 1, 1)
            assert config(source)["connections"][0]["category"] == "agent"
            app.quit()
            app = start(source)
            app.wait_text("Catalog Agent")
            app.select_tree("Catalog Agent")
            app.action("Edit")
            app.wait_text("HTTPS Runtime URL")
            app.fill("Name", "Agent Renamed", replace=True)
            app.modal_click("[Save]")
            app.wait(lambda: config(source)["connections"][0]["name"] == "Agent Renamed", "Agent blank-credential edit")
            assert config(source)["connections"][0]["category"] == "agent"
            app.idle()
            app.select_tree("Agent Renamed")
            app.action("Test")
            app.wait_text("主機連線正常")
            app.quit()
            passed("T-CRUD", "Mouse-created Agent category survives restart and blank-credential edit with real authentication")
            result = {"result": "PASS", "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                      "source_base_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=Path(__file__).resolve().parents[2]).decode().strip(),
                      "source_worktree_dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=Path(__file__).resolve().parents[2]).strip()),
                      "driver_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                      "checks": checks, "not_run": ["Physical Windows/APK", "Real AI models", "Owner private endpoints"]}
            (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        finally:
            errors = []
            for index, client in enumerate(clients):
                try:
                    client.save(output, "terminal-" + str(index))
                except Exception as error:
                    errors.append(error)
                try:
                    client.close()
                except Exception as error:
                    errors.append(error)
            if screen is not None:
                try:
                    screen.close()
                except Exception as error:
                    errors.append(error)
            if runtime is not None:
                try:
                    runtime.close()
                except Exception as error:
                    errors.append(error)
            if errors:
                raise ExceptionGroup("Catalog fixture cleanup failed", errors)
    print("All catalog, transfer, mouse and editable-keybinding checks passed.")


if __name__ == "__main__":
    main()
