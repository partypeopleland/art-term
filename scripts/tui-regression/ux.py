#!/usr/bin/env python3
"""Drive the Linux TUI connection-settings modal, explicit saved-record targets,
leader pending hints and custom keybindings through the real PTY."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

from catalog import CatalogTerminal, config, raw_config, wait_counts
from run import McpFixture, Runtime

SETTINGS = "[連線設定]"
MODAL = "連線設定 / "


def settings_open(app):
    app.click_text(SETTINGS, (0, 0, app.screen.columns, 1))
    app.wait_text(MODAL)
    app.settle()


def settings_close(app):
    app.modal_click("[Back]")
    app.wait(lambda: MODAL not in app.text(), "settings modal closed")


def settings_row(app, label):
    app.modal_click(label)
    for _ in range(4):
        app.pump()


def settings_key_row(app, index):
    # Clamp to the first row, then move to the requested flat row index.
    for _ in range(24):
        app.send(b"\x1b[A")
    app.settle(0.1)
    for _ in range(index):
        app.send(b"\x1b[B")
        app.settle(0.03)
    app.send(b"\r")
    for _ in range(4):
        app.pump()


def save_host_form(app, name, url, credential):
    app.wait_text("HTTPS Runtime URL")
    app.fill("Name", name)
    app.fill("HTTPS Runtime URL", url)
    app.fill("Credential", credential)
    app.modal_click("[Save]")


def save_screen_form(app, name, url, token):
    app.wait_text("HTTPS MCP URL")
    app.fill("Name", name)
    app.fill("HTTPS MCP URL", url)
    app.fill("Token", token)
    app.modal_click("[Save]")


def export_form(app, path, password):
    app.wait_text("Confirm password")
    app.fill("File path", str(path))
    app.fill("File password", password)
    app.fill("Confirm password", password)
    app.modal_click("[Save]")


def settings_import(app, path, password):
    settings_row(app, "Import / 匯入連線")
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

    with tempfile.TemporaryDirectory(prefix="art-term-tui-ux-") as directory:
        root = Path(directory)
        home_a = root / "home-a"
        home_b = root / "home-b"
        fixture = root / "fixture"
        for path in (home_a, home_b, fixture):
            path.mkdir(mode=0o700)
        runtime = Runtime(fixture)
        screen = McpFixture("ux")
        transfer = root / "ux-transfer.enc"
        file_password = "synthetic-ux-transfer-password"
        try:
            app = start(home_a)
            app.wait_text("Computer")
            app.settle()
            # Red anchor: the named settings entry must be visible at startup.
            assert app.locations(SETTINGS, (0, 0, app.screen.columns, 1)), \
                "No visible 連線設定 entry at startup"
            settings_open(app)
            for label in ("New Host", "New Agent", "New Screen", "Edit target",
                          "Test target", "Delete target", "Import", "Export", "Back"):
                app.wait_text(label)
            region = app.modal_region()
            modal_text = "\n".join(
                "".join(app.screen.buffer[y][x].data
                        for x in range(region[0], region[2]))
                for y in range(region[1], region[3]))
            assert "未綁定" not in modal_text and "→" not in modal_text, \
                "Settings advertises global shortcuts the modal cannot receive"
            assert "Enter" in modal_text and "Esc" in modal_text, \
                "Settings lacks an Enter/click activation hint"
            settings_close(app)
            # Keyboard path: F10 opens Actions, first row is connection settings.
            app.send(b"\x1b[21~")
            app.wait_text("Actions / ")
            app.send(b"\r")
            app.wait_text(MODAL)
            app.send(b"\x1b")
            app.wait(lambda: MODAL not in app.text(), "settings closed by Esc")
            app.settle(0.3)  # let the bare-ESC event settle before the next escape sequence
            # Create a Host through the settings modal; save returns to settings.
            settings_open(app)
            settings_row(app, "New Host / 新增 Host")
            save_host_form(app, "UX Host", runtime.origin, runtime.credential)
            app.wait_text(MODAL)
            app.wait_text("UX Host")
            wait_counts(app, home_a, 1, 0)
            settings_close(app)
            passed("T-UX", "Visible toolbar/Actions/F10 entry opens grouped 連線設定; save returns to settings")

            # Open a remote Shell so a Shell tree row exists.
            app.click_text("New Shell", (0, 1, 40, 24))
            app.wait(lambda: len(runtime.work()) == 1, "UX shell created")
            work = next(iter(runtime.work()))
            app.wait(lambda: runtime.work()[work]["bootstrap"], "UX shell bootstrap")
            app.wait_text("Live")
            app.idle()
            app.settle(0.8)
            app.select_tree(work)
            assert work in app.selected_tree(), "Shell row not selectable"

            # Settings opened on a Shell row must not imply a saved-record target.
            settings_open(app)
            assert "需先選擇連線" in app.text(), "No-target settings did not disable CRUD"
            settings_row(app, "Edit target")
            app.settle()
            assert "編輯 Host" not in app.text(), "Edit opened without a saved target"
            assert "請先選擇" in app.text(), "Disabled Edit gave no explanation"
            settings_key_row(app, 5)  # Delete target row (actions are listed first)
            app.wait_text("請先選擇")
            assert "確認 / Confirm" not in app.text(), "Delete confirmed without target"
            events = runtime.snapshot()["events"]
            deletes = [e for e in events
                       if e.get("method") == "DELETE" and (e.get("name") or "").startswith("th2-")]
            assert not deletes, f"Settings delete touched the remote Shell: {deletes}"
            settings_close(app)
            passed("T-UX", "Shell-row settings has explicit no-target state; Edit/Delete stay local and disabled")

            # Create a Screen through settings; it becomes the typed target.
            settings_open(app)
            settings_row(app, "New Screen / 新增 Screen")
            save_screen_form(app, "UX Screen", screen.url, screen.token)
            app.wait_text(MODAL)
            wait_counts(app, home_a, 1, 1)
            app.wait_text("UX Screen")
            settings_close(app)

            # Screen target while the Shell tab stays active: delete only the record.
            app.select_tree("UX Screen")
            assert "UX Screen" in app.selected_tree()
            settings_open(app)
            app.wait_text("Screen：UX Screen")
            settings_row(app, "Delete target")
            app.wait_text("確認 / Confirm")
            app.wait_text("刪除此 Screen")
            app.modal_click("[Yes]")
            app.wait_text(MODAL)
            wait_counts(app, home_a, 1, 0)
            assert len(runtime.work()) == 1, "Screen delete harmed the remote Shell"
            assert not any(e.get("method") == "DELETE"
                           and (e.get("name") or "").startswith("th2-")
                           for e in runtime.snapshot()["events"]), \
                "Screen-target delete sent a remote DELETE"
            assert "UX Host" in app.text()
            passed("T-UX", "Screen target deletes only the saved record; active Shell tab and remote work survive")

            # Recreate a Screen so the encrypted export really mixes Host and Screen,
            # and verify actions stay above the saved-record list.
            settings_open(app)
            settings_row(app, "New Screen / 新增 Screen")
            save_screen_form(app, "UX Screen", screen.url, screen.token)
            app.wait_text(MODAL)
            wait_counts(app, home_a, 1, 1)
            region = app.modal_region()
            imp = app.locations("Import /", region)
            tgt = app.locations("Host：", region)
            assert imp and tgt and imp[0][1] < tgt[0][1], \
                "Import/Export buried below saved connections"
            settings_close(app)

            # Host target: keyboard Test then mouse Edit, both return to settings.
            app.select_tree("UX Host")
            settings_open(app)
            settings_key_row(app, 4)  # Test target row
            app.wait_text("主機連線正常")
            app.wait_text(MODAL)
            settings_row(app, "Edit target")
            app.wait_text("編輯 Host")
            app.fill("Name", "UX Host Renamed", replace=True)
            app.modal_click("[Save]")
            app.wait_text(MODAL)
            app.wait(lambda: config(home_a)["connections"][0]["name"] == "UX Host Renamed",
                     "settings edit saved")
            app.wait_text("UX Host Renamed")
            settings_close(app)
            passed("T-UX", "Host target Test/Edit act on the displayed record and return to settings")

            # Editing the Host bumped its revision and detached the Shell tab;
            # the work is retained remotely, so re-attach it explicitly.
            app.select_tree(work)
            app.send("\r")
            app.wait_text("確認 / Confirm")
            app.send("y")
            app.wait_text("Live")
            app.idle()

            # Leader pending legend, Esc cancellation and double-leader forwarding.
            app.click_text("[Terminal]", (0, 0, app.screen.columns, 1))
            app.wait_text("Live")
            app.send("UX-REMOTE-BASELINE")
            app.wait(lambda: "UX-REMOTE-BASELINE" in runtime.work()[work]["input"],
                     "remote baseline input")
            app.send(b"\x1d")
            app.wait(lambda: "命令模式" in app.text().splitlines()[-1],
                     "leader pending hint")
            footer = app.text().splitlines()[-1]
            assert "Esc" in footer and "取消" in footer, "Leader pending lacks cancel hint"
            assert "連線" in footer or "說明" in footer or "終端" in footer, \
                "Leader pending does not list next actions"
            frames_before = len(runtime.frames)
            app.send(b"\x1b")
            app.wait(lambda: "命令模式" not in app.text().splitlines()[-1],
                     "Esc cancels leader pending")
            assert len(runtime.frames) == frames_before, "Pending cancel leaked to remote"
            app.send(b"\x1d\x1d")
            app.wait(lambda: "\x1d" in runtime.work()[work]["input"],
                     "double leader forwards to remote", 5)
            app.send(b"\x1dz")
            app.settle(0.2)
            assert "未知快捷鍵" in app.text(), "Unknown leader chord gave no feedback"
            passed("T-UX", "Leader pending shows configured next actions plus Esc cancel; double leader still forwards")

            # Encrypted export through settings, then import into a fresh home.
            settings_open(app)
            settings_row(app, "Export / 匯出全部連線")
            export_form(app, transfer, file_password)
            app.wait_text(MODAL)
            app.wait(lambda: transfer.exists(), "settings export written")
            app.wait_text("已加密匯出")
            for secret in (runtime.credential, screen.token, file_password):
                assert secret.encode() not in transfer.read_bytes()
                assert secret.encode() not in app.raw
            assert transfer.stat().st_mode & 0o777 == 0o600
            settings_close(app)
            app.quit()
            passed("T-UX", "Settings export writes an encrypted mixed Host/Screen file without leaking secrets")

            restored = start(home_b)
            settings_open(restored)
            before = raw_config(home_b)
            settings_import(restored, transfer, "wrong-ux-password")
            restored.wait(lambda: "Authentication" in "".join(restored.text().split())
                          or "rejected" in "".join(restored.text().split()),
                          "wrong-password rejection", 15)
            restored.wait_text(MODAL)
            settings_import(restored, transfer, file_password)
            restored.wait_text("Import preview")
            restored.modal_click("[Cancel]")
            restored.wait_text(MODAL)
            assert raw_config(home_b) == before
            settings_import(restored, transfer, file_password)
            restored.wait_text("Import preview")
            restored.modal_click("[Apply]")
            restored.wait_text(MODAL)
            wait_counts(restored, home_b, 1, 1)
            restored.wait_text("UX Host Renamed")
            restored.wait_text("UX Screen")
            passed("T-UX", "Settings import preview cancel preserves bytes; apply round-trips and returns to settings")

            # Restart proves imported credentials still authenticate.
            settings_close(restored)
            restored.quit()
            restored = start(home_b)
            restored.wait_text("UX Host Renamed")
            settings_open(restored)
            settings_row(restored, "UX Host Renamed")  # explicit saved-record target
            settings_key_row(restored, 4)  # Test target row
            restored.wait_text("主機連線正常")
            restored.wait_text(MODAL)
            settings_close(restored)
            passed("T-UX", "Restarted import authenticates; settings Test uses the typed target")
            restored.quit()  # release the home_b vault lock before the custom-bindings run

            # Custom keybindings: changed leader, disabled F1, user-assigned F8.
            bindings = home_b / "config/art-term-tui/keybindings.toml"
            bindings.parent.mkdir(parents=True, exist_ok=True)
            custom = ('leader = "Ctrl+g"\n'
                      '[bindings]\nhelp = ["F9"]\nsettings = ["F8"]\nhosts = []\n')
            bindings.write_text(custom)
            before_bytes = bindings.read_bytes()
            custom_app = start(home_b)
            custom_app.send(b"\x1bOP")  # F1 must be disabled
            for _ in range(5):
                custom_app.pump()
            assert "Help / " not in custom_app.text(), "Disabled F1 still opened Help"
            custom_app.send(b"\x1b[19~")  # F8 opens settings
            custom_app.wait_text(MODAL)
            custom_app.send(b"\x1b")  # modal input is exclusive; close before global Help
            custom_app.wait(lambda: MODAL not in custom_app.text(), "settings closed")
            custom_app.settle(0.3)
            custom_app.send(b"\x1b[20~")  # F9 opens Help
            custom_app.wait_text("Help / ")
            custom_app.wait_text("Ctrl+g")
            custom_app.wait_text("F8")
            custom_app.click_text("[Close]")
            custom_app.wait(lambda: "Help / " not in custom_app.text(), "custom Help closed")
            custom_app.quit()
            assert bindings.read_bytes() == before_bytes, "TUI rewrote keybindings.toml"
            passed("T-UX", "Custom leader/disabled Help/user F8 settings work; config bytes preserved")

            # Narrow layout: settings reachable and cancellable at 20x8.
            app = start(home_a)
            app.resize(8, 20)
            app.wait_text("[Actions]")
            app.wait_text("[Desktop]")
            app.send(b"\x1b[21~")  # F10
            app.wait_text("Actions / ")
            app.send(b"\r")  # first row: 連線設定
            app.wait_text(MODAL)
            app.settle()
            region = app.modal_region()
            assert app.locations("[Back]", region) or app.locations("[Cancel]", region), \
                "No return control at 20x8"
            # Keyboard reach Import, cancel; Export must fill every secret field by mouse.
            settings_key_row(app, 6)  # Import row (actions listed first)
            app.wait_text("[Save]")
            app.send(b"\x1b")
            app.wait_text(MODAL)
            app.settle(0.3)
            app.send(b"\x1b[B")
            app.settle(0.03)
            app.send(b"\r")
            app.wait_text("[Save]")
            narrow_file = root / "ux-narrow.enc"
            region = app.modal_region()
            for row, value in enumerate(
                    (str(narrow_file), "narrow-pass-1", "narrow-pass-1")):
                app.click(region[0] + 2, region[1] + row)
                for _ in range(3):
                    app.pump()
                app.send(value.encode())
            app.settle(0.2)
            app.click(*app.locations("[Save]", region)[-1])
            for _ in range(4):
                app.pump()
            app.wait(lambda: narrow_file.exists(), "narrow export file written")
            assert narrow_file.stat().st_mode & 0o777 == 0o600
            app.wait_text(MODAL)  # settings origin restored after export
            app.send(b"\x1b[B")  # Back row
            app.settle(0.03)
            app.send(b"\r")
            app.wait(lambda: MODAL not in app.text(), "Back row closed settings")
            app.resize(36, 120)
            app.wait_text("[Connections]")
            app.quit()
            passed("T-UX", "20x8 keeps settings, Import/Export forms and Back reachable by keyboard")

            result = {"result": "PASS",
                      "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                      "source_base_commit": subprocess.check_output(
                          ["git", "rev-parse", "HEAD"],
                          cwd=Path(__file__).resolve().parents[2]).decode().strip(),
                      "source_worktree_dirty": bool(subprocess.check_output(
                          ["git", "status", "--porcelain"],
                          cwd=Path(__file__).resolve().parents[2]).strip()),
                      "driver_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                      "checks": checks,
                      "not_run": ["Physical Windows/APK", "Real AI models", "Owner private endpoints"]}
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
            for fixture_obj in (screen, runtime):
                if fixture_obj is not None:
                    try:
                        fixture_obj.close()
                    except Exception as error:
                        errors.append(error)
            if errors:
                raise ExceptionGroup("UX fixture cleanup failed", errors)
    print("All Linux TUI UX checks passed.")


if __name__ == "__main__":
    main()
