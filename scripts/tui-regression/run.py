#!/usr/bin/env python3
"""Drive the real terminal client against an isolated TLS/WSS Runtime fixture."""
import argparse
import asyncio
import codecs
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

import pyte

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "windows-regression"))
from fixture import Fixture


class Screen(pyte.Screen):
    def set_mode(self, *modes, **kwargs):
        if kwargs.get("private") and any(mode in (47, 1047, 1049) for mode in modes):
            self.erase_in_display(2)
            self.cursor_position()
        super().set_mode(*modes, **kwargs)


class Terminal:
    def __init__(self, binary, home, ca=None, rows=36, columns=120):
        self.master, self.slave = pty.openpty()
        self.original = termios.tcgetattr(self.slave)
        self.screen = Screen(columns, rows)
        self.stream = pyte.Stream(self.screen)
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.raw = bytearray()
        self.resize(rows, columns)
        env = os.environ.copy()
        for name in ("DISPLAY", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS", "DBUS_SYSTEM_BUS_ADDRESS", "ART_TERM_OPENAB_FIXTURE_CA"):
            env.pop(name, None)
        env.update(HOME=str(home), XDG_CONFIG_HOME=str(home / "config"), XDG_DATA_HOME=str(home / "data"), XDG_CACHE_HOME=str(home / "cache"), TERM="xterm-256color", COLORTERM="truecolor", LC_ALL="C.UTF-8")
        if ca is not None:
            env["ART_TERM_OPENAB_FIXTURE_CA"] = str(ca)

        def controlling_terminal():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)

        self.process = subprocess.Popen([str(binary)], stdin=self.slave, stdout=self.slave, stderr=self.slave, env=env, preexec_fn=controlling_terminal, close_fds=True)

    def resize(self, rows, columns):
        fcntl.ioctl(self.master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
        self.screen.resize(rows, columns)

    def pump(self, timeout=0.05):
        if select.select([self.master], [], [], timeout)[0]:
            try:
                data = os.read(self.master, 65536)
            except OSError:
                return
            self.raw.extend(data)
            self.stream.feed(self.decoder.decode(data))
            if b"\x1b[6n" in data:
                self.send(b"\x1b[1;1R")

    def text(self):
        return "\n".join(
            "".join(self.screen.buffer[row][column].data for column in range(self.screen.columns))
            for row in range(self.screen.lines)
        )

    def wait(self, predicate, message, timeout=25):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.pump()
            if predicate():
                return
            if self.process.poll() is not None:
                raise AssertionError(f"Client exited during {message}: {self.process.returncode}\n{self.text()}")
        raise AssertionError(f"Timed out during {message}\n{self.text()}")

    def wait_text(self, text, timeout=25):
        self.wait(lambda: text in self.text(), text, timeout)

    def send(self, data):
        if isinstance(data, str):
            data = data.encode()
        os.write(self.master, data)

    def prefix(self, command):
        self.send(b"\x1d" + command.encode())

    def unlock(self, password, fresh=False):
        self.wait_text("New vault password:" if fresh else "Vault password:")
        assert not termios.tcgetattr(self.slave)[3] & termios.ECHO
        self.send(password + "\n")
        if fresh:
            self.wait_text("Confirm password:")
            self.send(password + "\n")
        self.wait_text("Hosts")

    def finish(self, expected=0):
        deadline = time.monotonic() + 15
        while self.process.poll() is None and time.monotonic() < deadline:
            self.pump()
        assert self.process.poll() is not None, "Client did not exit"
        self.pump(0)
        assert self.process.returncode == expected, (self.process.returncode, self.text())
        current = termios.tcgetattr(self.slave)
        mask = termios.ECHO | termios.ICANON | termios.ISIG
        assert current[3] & mask == self.original[3] & mask, "TTY mode was not restored"

    def save(self, output, name):
        (output / (name + ".txt")).write_text(self.text())
        (output / (name + ".ansi")).write_bytes(self.raw)

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        os.close(self.master)
        os.close(self.slave)


class Runtime(Fixture):
    def __init__(self, root):
        self.frames = []
        self.query_during_follow = False
        self.follow_query_verified = False
        super().__init__(root)

    def observe_input(self, name, data):
        if name.startswith("th2-"):
            self.frames.append((name, bytes(data)))

    async def history_request(self, shell):
        line = shell["previewInput"].split(b"\n", 1)[0]
        if self.query_during_follow and b"\n" in shell["previewInput"]:
            request = json.loads(line)
            if request.get("op") == "follow" and "input" in request:
                self.query_during_follow = False
                name = shell["historyWork"]
                start = len(self.frames)
                await self.output(self.sessions[name], b"\x1b[2;3H\x1b[6n")
                deadline = asyncio.get_running_loop().time() + 5
                while asyncio.get_running_loop().time() < deadline:
                    if any(tab == name and b"\x1b[2;3R" in data for tab, data in self.frames[start:]):
                        self.follow_query_verified = True
                        break
                    await asyncio.sleep(0.01)
                assert self.follow_query_verified, "Terminal query stalled while atomic follow awaited its reply"
        await super().history_request(shell)

    def emit(self, name, data):
        self.call(self.output(self.sessions[name], data))

    def work(self):
        return {name: state for name, state in self.snapshot()["sessions"].items() if name.startswith("th2-")}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True, help="Debug binary built with openab-fixture")
    parser.add_argument("--output-dir", type=Path, required=True, help="New output directory")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=False)
    checks = []

    def passed(check):
        checks.append(check)
        print("PASS " + check, flush=True)

    terminals = []
    runtime = None
    with tempfile.TemporaryDirectory(prefix="art-term-tui-test-") as directory:
        root = Path(directory)
        home = root / "home"
        home.mkdir(mode=0o700)
        fixture_root = root / "fixture"
        fixture_root.mkdir(mode=0o700)
        runtime = Runtime(fixture_root)
        password = "fixture-vault-password-Only"
        try:
            app = Terminal(binary, home, runtime.ca)
            terminals.append(app)
            app.unlock(password, fresh=True)
            app.wait_text("No Hosts")
            app.send("n")
            app.wait_text("HTTPS Runtime URL")
            app.send("PTY fixture\t" + runtime.origin + "\t" + runtime.credential)
            app.send(b"\x13")
            app.wait_text("PTY fixture")
            app.wait(lambda: (home / "config/art-term-tui/connections.toml").exists(), "Host persistence")
            config = home / "config/art-term-tui/connections.toml"
            assert runtime.credential not in config.read_text()
            assert runtime.credential.encode() not in app.raw
            assert password.encode() not in app.raw
            assert config.stat().st_mode & 0o777 == 0o600
            app.wait(lambda: "1 Hosts" in app.text() and "Working" not in app.text() and "HTTPS Runtime URL" not in app.text(), "saved Host list ready")
            passed("Headless vault and masked Host form with encrypted persistence")

            app.prefix("s")
            app.wait_text("遠端 Shells")
            app.send("n")
            app.wait(lambda: len(runtime.work()) == 1, "first remote work creation")
            first = next(iter(runtime.work()))
            app.wait(lambda: runtime.work()[first]["bootstrap"], "history bootstrap")
            app.wait_text("1:")
            app.wait_text("Live")
            app.send("FIRST-INPUT")
            app.wait(lambda: "FIRST-INPUT" in runtime.work()[first]["input"], "first input")
            runtime.emit(first, "\r\n\x1b[31mANSI-繁體中文\x1b[0m\r\n".encode())
            app.wait_text("ANSI-繁體中文")
            app.save(output, "first-tab")
            passed("Real TLS/WSS work creation, one-time bootstrap, UTF-8 and ANSI")

            app.prefix("n")
            app.wait(lambda: len(runtime.work()) == 2, "second remote work creation")
            second = next(name for name in runtime.work() if name != first)
            app.wait(lambda: runtime.work()[second]["bootstrap"], "second bootstrap")
            app.wait_text("2:")
            app.wait_text("Live")
            app.send("SECOND-INPUT")
            app.wait(lambda: "SECOND-INPUT" in runtime.work()[second]["input"], "second input")
            runtime.emit(first, b"\r\nBACKGROUND-FIRST\r\n")
            runtime.emit(second, b"\r\nFOREGROUND-SECOND\r\n")
            app.wait_text("FOREGROUND-SECOND")
            app.prefix("1")
            app.wait_text("BACKGROUND-FIRST")
            assert "SECOND-INPUT" not in runtime.work()[first]["input"]
            assert "FIRST-INPUT" not in runtime.work()[second]["input"]
            passed("Two tabs isolate input, retain background output and switch screens")

            app.resize(42, 132)
            app.wait(lambda: all(state["resizes"] and state["resizes"][-1] == [39, 132] for state in runtime.work().values()), "all-tab resize excludes UI chrome")
            runtime.append_lines(first, 0, 100)
            app.prefix("u")
            app.wait(lambda: any(event.get("method") == "history" and event.get("action") == "scroll" and event.get("lines") == -12 for event in runtime.snapshot()["events"]), "native history scroll")
            runtime.query_during_follow = True
            app.send("Z")
            app.wait(lambda: runtime.follow_query_verified, "query while follow is pending")
            app.wait(lambda: runtime.work()[first]["input"].endswith("Z"), "atomic follow input")
            assert runtime.work()[first]["input"].count("Z") == 1
            passed("Shared history scroll/follow ordering and VT query during pending follow")

            before = runtime.snapshot()
            attaches = len(before["sessions"][first]["attaches"])
            posts = [event for event in before["events"] if event["method"] == "POST" and (event["name"] or "").startswith("th2-")]
            runtime.disconnect(first)
            app.wait(lambda: len(runtime.work()[first]["attaches"]) > attaches, "same-token recovery")
            after = runtime.snapshot()
            resumed = after["sessions"][first]["attaches"][-1]
            previous = before["sessions"][first]["attaches"][-1]
            assert resumed["tokenHash"] == previous["tokenHash"]
            assert resumed["since"] > 0
            assert [event for event in after["events"] if event["method"] == "POST" and (event["name"] or "").startswith("th2-")] == posts
            assert len(runtime.work()) == 2
            assert after["sessions"][first]["input"].count("Z") == 1
            passed("Same-token cursor recovery without work create/renew or input replay")

            app.prefix("w")
            app.wait(lambda: first in runtime.work() and not runtime.work()[first]["attached"], "local detach")
            assert not any(event["method"] == "DELETE" and event["name"] == first for event in runtime.snapshot()["events"])

            first_generation = runtime.work()[first]["generation"]
            app.prefix("s")
            app.wait_text("遠端 Shells")
            app.send("\r")
            app.wait_text("確認 / Confirm")
            app.send("y")
            app.wait(lambda: runtime.work()[first]["generation"] == first_generation + 1 and runtime.work()[first]["attached"], "explicit existing Shell attach")
            app.wait_text("2:")
            app.wait_text("Live")
            app.send("REATTACHED-FIRST")
            app.wait(lambda: "REATTACHED-FIRST" in runtime.work()[first]["input"], "reattached Shell input")

            second_generation = runtime.work()[second]["generation"]
            app.prefix("s")
            app.wait_text("遠端 Shells")
            app.send(b"\x1b[B\r")
            app.wait_text("確認 / Confirm")
            app.wait_text("接管")
            app.send("y")
            app.wait(lambda: runtime.work()[second]["generation"] == second_generation + 1 and runtime.work()[second]["attached"], "explicit attached Shell takeover")
            app.wait_text("3:")
            app.wait_text("Live")
            app.send("TAKEN-SECOND")
            app.wait(lambda: "TAKEN-SECOND" in runtime.work()[second]["input"], "takeover input")
            assert len(runtime.work()) == 2
            passed("Existing Shell attach and explicit attached Shell takeover through confirmation UI")

            app.prefix("s")
            app.wait_text("遠端 Shells")
            app.send("d")
            app.wait_text("確認 / Confirm")
            app.send("y")
            app.wait(lambda: first not in runtime.work(), "explicit remote deletion")
            assert second in runtime.work()
            assert sum(event["method"] == "DELETE" and event["name"] == first for event in runtime.snapshot()["events"]) == 1
            passed("Confirmed remote deletion is separate from local tab detach")
            app.prefix("q")
            app.finish()
            app.save(output, "normal-exit")
            passed("Local tab close preserves work Shell; normal exit restores TTY")

            restarted = Terminal(binary, home, runtime.ca)
            terminals.append(restarted)
            restarted.unlock(password)
            restarted.wait_text("PTY fixture")
            restarted.process.send_signal(signal.SIGTERM)
            restarted.finish()
            restarted.save(output, "restart-sigterm")
            passed("Restart unlock and signal exit restore TTY")

            wrong = Terminal(binary, home, runtime.ca)
            terminals.append(wrong)
            original = config.read_bytes()
            wrong.wait_text("Vault password:")
            wrong.send("wrong-fixture-password\n")
            wrong.finish(expected=1)
            assert config.read_bytes() == original
            passed("Wrong password preserves saved data and restores echo")

            interrupted = Terminal(binary, home, runtime.ca)
            terminals.append(interrupted)
            interrupted.wait_text("Vault password:")
            interrupted.process.send_signal(signal.SIGTERM)
            interrupted.finish(expected=143)
            passed("SIGTERM during hidden password prompt restores TTY")
            (output / "fixture.json").write_text(json.dumps(runtime.snapshot(), indent=2) + "\n")
            (output / "result.json").write_text(json.dumps({"result": "PASS", "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "checks": checks}, indent=2) + "\n")
            print("All native terminal checks passed.")
        finally:
            cleanup_errors = []
            for index, terminal in enumerate(terminals):
                try:
                    terminal.save(output, f"terminal-{index}")
                except Exception as error:
                    cleanup_errors.append(error)
                try:
                    terminal.close()
                except Exception as error:
                    cleanup_errors.append(error)
            try:
                runtime.close()
            except Exception as error:
                cleanup_errors.append(error)
            if cleanup_errors:
                raise ExceptionGroup("Terminal fixture cleanup failed", cleanup_errors)


if __name__ == "__main__":
    main()
