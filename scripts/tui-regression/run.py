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
import re
import select
import signal
import ssl
import struct
import subprocess
import sys
import tempfile
import termios
import time
import tomllib
import zlib

import aiohttp
import pyte

sys.path.insert(0, str(Path(__file__).resolve().parent))
from native_images import NativeImages

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "windows-regression"))
from fixture import Fixture
from mcp_fixture import McpFixture


def screenshot_png(primary):
    def chunk(kind, data):
        return struct.pack("!I", len(data)) + kind + data + struct.pack("!I", zlib.crc32(kind + data) & 0xffffffff)
    width, height = 80, 40
    other = (30, 220, 60)
    rows = b"".join(b"\0" + b"".join(bytes(primary if (x < 40) == (y < 20) else other)
                                           for x in range(width)) for y in range(height))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack("!IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


class Screen(pyte.Screen):
    images = None

    def erase_in_display(self, how=0, *args, **kwargs):
        if self.images is not None and how in (0, 1, 2, 3):
            self.images.clear()
        super().erase_in_display(how, *args, **kwargs)

    def set_mode(self, *modes, **kwargs):
        if kwargs.get("private") and any(mode in (47, 1047, 1049) for mode in modes):
            self.erase_in_display(2)
            self.cursor_position()
        super().set_mode(*modes, **kwargs)


class Terminal:
    def __init__(self, binary, home, ca=None, rows=36, columns=120, env_overrides=None, cell_pixels=(8, 16), terminal_reports=None, tty_pixels=None):
        self.cell_pixels = cell_pixels
        self.tty_pixels = tty_pixels
        self.terminal_reports = terminal_reports or {}
        self.probe_queries = []
        self._query_scanned = self._query_end = 0
        self._scheduled_reports = []
        self.master, self.slave = pty.openpty()
        self.original = termios.tcgetattr(self.slave)
        self.screen = Screen(columns, rows)
        self.stream = pyte.Stream(self.screen)
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.raw = bytearray()
        self.images = NativeImages(self.screen, self.stream, self.decoder, cell_pixels)
        self.screen.images = self.images
        self.sixels = self.images.payloads
        self.resize(rows, columns)
        env = os.environ.copy()
        for name in ("NO_COLOR", "DISPLAY", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS", "DBUS_SYSTEM_BUS_ADDRESS", "ART_TERM_OPENAB_FIXTURE_CA", "SSH_CLIENT", "SSH_CONNECTION", "SSH_TTY", "TMUX", "STY"):
            env.pop(name, None)
        env.update(HOME=str(home), XDG_CONFIG_HOME=str(home / "config"), XDG_DATA_HOME=str(home / "data"), XDG_CACHE_HOME=str(home / "cache"), TERM="vt340", XTERM_VERSION="XTerm(390)", COLORTERM="truecolor", LC_ALL="C.UTF-8")
        if env_overrides:
            for name, value in env_overrides.items():
                if value is None: env.pop(name, None)
                else: env[name] = value
        if ca is not None:
            env["ART_TERM_OPENAB_FIXTURE_CA"] = str(ca)

        def controlling_terminal():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)

        self.process = subprocess.Popen([str(binary)], stdin=self.slave, stdout=self.slave, stderr=self.slave, env=env, preexec_fn=controlling_terminal, close_fds=True)

    def resize(self, rows, columns):
        fcntl.ioctl(self.master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, *(self.tty_pixels or (columns * self.cell_pixels[0], rows * self.cell_pixels[1]))))
        self.screen.resize(rows, columns)

    def pump(self, timeout=0.05):
        self._send_due_reports()
        if select.select([self.master], [], [], timeout)[0]:
            try:
                data = os.read(self.master, 65536)
            except OSError:
                return
            self.raw.extend(data)
            self.images.feed(data)
            self._answer_probes()
            if b"\x1b[6n" in data:
                self.send(b"\x1b[1;1R")

    def _send_due_reports(self):
        now = time.monotonic()
        due = [entry for entry in self._scheduled_reports if entry[0] <= now]
        self._scheduled_reports = [entry for entry in self._scheduled_reports if entry[0] > now]
        for _, data in sorted(due, key=lambda entry: entry[0]):
            self.send(data)

    def _answer_probes(self):
        width, height = self.cell_pixels
        defaults = {
            "c": b"\x1b[?61;4;6;7c",
            "16t": f"\x1b[6;{height};{width}t".encode(),
            "14t": f"\x1b[4;{self.screen.lines * height};{self.screen.columns * width}t".encode(),
            "18t": f"\x1b[8;{self.screen.lines};{self.screen.columns}t".encode(),
        }
        start = max(0, self._query_scanned - 8)
        for match in re.finditer(rb"\x1b\[(c|16t|14t|18t)", self.raw[start:]):
            end = start + match.end()
            if end <= self._query_end:
                continue
            self._query_end = end
            name = match[1].decode()
            self.probe_queries.append(name)
            response = self.terminal_reports.get(name, defaults[name])
            if response is None:
                continue
            parts = [(0, response)] if isinstance(response, bytes) else response
            now = time.monotonic()
            self._scheduled_reports.extend((now + delay, data) for delay, data in parts)
        self._query_scanned = len(self.raw)
        self._send_due_reports()

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

    def start(self):
        self.wait_text("Connections")
        assert b"Vault password:" not in self.raw
        assert b"New vault password:" not in self.raw

    def idle(self):
        self.wait(lambda: "Working" not in self.text(), "management job completion")
        for _ in range(4):
            self.pump()

    def shells(self):
        self.prefix("s")
        self.wait_text("Refresh Shells")
        self.idle()

    def action(self, label):
        self.click_text("[Actions]")
        self.wait_text("Actions / ")
        width, height = min(self.screen.columns - 4, 88), min(self.screen.lines - 4, 24)
        left, top = (self.screen.columns - width) // 2, (self.screen.lines - height) // 2
        region = (left + 1, top + 1, left + width - 1, top + height - 1)
        for _ in range(30):
            hits = self.locations(label, region)
            if hits:
                self.click(*hits[0])
                for _ in range(3): self.pump()
                return
            self.wheel(left + width // 2, top + height // 2, 1)
        raise AssertionError("Action is not mouse accessible: " + label)

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

    def locations(self, label, region=None):
        hits = []
        for y in range(self.screen.lines):
            chars, columns = [], []
            for x in range(self.screen.columns):
                for char in self.screen.buffer[y][x].data:
                    chars.append(char)
                    columns.append(x)
            text = "".join(chars)
            start = 0
            while (position := text.find(label, start)) >= 0:
                x = columns[position + len(label) // 2]
                if region is None or region[0] <= x < region[2] and region[1] <= y < region[3]:
                    hits.append((x, y))
                start = position + len(label)
        return hits

    def click(self, x, y):
        self.send(f"\x1b[<0;{x + 1};{y + 1}M\x1b[<0;{x + 1};{y + 1}m")

    def click_text(self, label, region=None, occurrence=0):
        self.wait(lambda: len(self.locations(label, region)) > occurrence, "mouse target " + label)
        self.click(*self.locations(label, region)[occurrence])
        for _ in range(3):
            self.pump()

    def wheel(self, x, y, lines):
        button = 65 if lines > 0 else 64
        for _ in range(abs(lines)):
            self.send(f"\x1b[<{button};{x + 1};{y + 1}M")
        for _ in range(3):
            self.pump()

    def image_cells(self, color):
        return self.images.cells(color)

    def image_signature(self, region):
        return self.images.signature(region)

    def bind_screen(self, row):
        self.prefix("c")
        self.wait_text("Stop")
        self.send(b"\x1b[B" * row + b"\r")

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
        self.external_clients = []
        self.external_tasks = []
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

    async def attach_external(self, name):
        context = ssl.create_default_context(cadata=ssl.DER_cert_to_PEM_cert(self.ca.read_bytes()))
        client = aiohttp.ClientSession()
        self.external_clients.append(client)
        socket = await client.ws_connect(self.origin.replace("https://", "wss://") + "/pty/" + name,
                                headers={"Authorization": "Bearer " + self.sessions[name]["token"]}, ssl=context)
        async def receive():
            async for _ in socket:
                pass
        self.external_tasks.append(asyncio.create_task(receive()))

    def close(self):
        async def close_external():
            for client in self.external_clients:
                await client.close()
            await asyncio.gather(*self.external_tasks, return_exceptions=True)
        self.call(close_external())
        super().close()

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
        mcps = [McpFixture("A"), McpFixture("B", sse=True)]
        for mcp, color in zip(mcps, [(240, 30, 20), (20, 30, 240)]):
            mcp.image = screenshot_png(color)
        try:
            app = Terminal(binary, home, runtime.ca)
            terminals.append(app)
            app.start()
            app.click_text("New Host")
            app.wait_text("HTTPS Runtime URL")
            app.send("PTY fixture\t" + runtime.origin + "\t" + runtime.credential)
            app.send(b"\x13")
            app.wait_text("PTY fixture")
            app.wait(lambda: (home / "config/art-term-tui/connections.toml").exists(), "Host persistence")
            config = home / "config/art-term-tui/connections.toml"
            assert runtime.credential not in config.read_text()
            assert runtime.credential.encode() not in app.raw
            assert config.stat().st_mode & 0o777 == 0o600
            app.wait(lambda: "1 Hosts" in app.text() and "Working" not in app.text() and "HTTPS Runtime URL" not in app.text(), "saved Host list ready")
            key = home / "config/art-term-tui/connections.toml.account-key-v1"
            assert key.stat().st_mode & 0o777 == 0o600
            assert len(key.read_bytes()) == 39
            passed("Prompt-free headless startup and masked Host form with encrypted persistence")

            app.shells()
            app.click_text("New Shell", region=(0, 1, 40, 24))
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

            for held in (False, True):
                if held:
                    runtime.append_lines(first, 0, 100)
                    app.prefix("u")
                    app.wait_text("歷史閱讀")
                history_label = re.search(r"歷史閱讀\s*-\d+|即時輸出", app.text()).group()
                app.prefix("2")
                app.wait_text("FOREGROUND-SECOND")
                before = runtime.snapshot()
                app.select_tree(first)
                app.send(b"\r")
                app.settle()
                assert "確認 / Confirm" not in app.text(), "Selecting an existing local Shell requested takeover instead of switching tabs"
                assert "3:" not in app.text(), "Selecting an existing local Shell created another tab"
                assert history_label in app.text(), "Switching to a local Shell changed its history state"
                assert "歷史控制狀態不確定" not in app.text()
                after = runtime.snapshot()
                assert after["sessions"][first]["generation"] == before["sessions"][first]["generation"]
                assert [event for event in after["events"] if event["method"] in ("POST", "DELETE")] == [
                    event for event in before["events"] if event["method"] in ("POST", "DELETE")
                ], "Switching to a local Shell recreated a work or helper connection"
            app.send("LOCAL-REUSED")
            app.wait(lambda: runtime.work()[first]["input"].endswith("LOCAL-REUSED"), "input through reused local tab")
            assert runtime.work()[first]["input"].count("LOCAL-REUSED") == 1
            assert "LOCAL-REUSED" not in runtime.work()[second]["input"]
            passed("Existing local Shell selection reuses its tab and held history without renew or duplicate input")

            frames_before = len(runtime.frames)
            app.prefix("h")
            app.idle()
            for index, mcp in enumerate(mcps, 1):
                app.action("New Screen")
                app.wait_text("新增 Screen")
                app.send("Screen-" + mcp.name + "\t" + mcp.url + "\t" + mcp.token)
                app.send(b"\x13")
                app.wait(lambda: len(tomllib.loads(config.read_text()).get("desktops", [])) == index and "新增 Screen" not in app.text(), "Screen saved")
                app.idle()
            assert len(runtime.frames) == frames_before
            assert all(mcp.token not in config.read_text() and mcp.token.encode() not in app.raw for mcp in mcps)
            passed("Screen creation uses masked tokens and side-pane input never reaches remote work")

            app.prefix("1")
            app.bind_screen(1)
            app.wait(lambda: bool(app.image_cells("f01e14")), "real red MCP screenshot pixels", timeout=40)
            cells = app.image_cells("f01e14")
            assert all(1 <= x < 39 and 24 <= y < 33 for x, y in cells), cells
            runtime.emit(first, b"\r\nOUTPUT-WITH-IMAGE\r\n")
            app.wait_text("OUTPUT-WITH-IMAGE")
            app.save(output, "three-panes-real-image")
            app.wait(lambda: any(event.get("method") == "tools-control" and event.get("action") == "configure" and event.get("binding") == first for event in runtime.snapshot()["events"]), "selected Screen configures exact active Shell")
            app.wait_text("Tools unconfirmed")
            passed("Actual PNG screenshot becomes native Sixel pixels in lower-left while terminal output continues")
            passed("Actual Shell configure exchange occurs; zero-adapter fixture never claims tools ready")

            app.prefix("2")
            app.wait(lambda: not app.image_cells("f01e14"), "tab switch clears previous screenshot")
            app.bind_screen(2)
            app.wait(lambda: bool(app.image_cells("141ef0")), "real blue SSE MCP screenshot pixels", timeout=40)
            app.prefix("1")
            app.wait(lambda: bool(app.image_cells("f01e14")) and not app.image_cells("141ef0"), "first tab Screen binding restored", timeout=40)
            app.prefix("v")
            app.wait(lambda: bool(app.image_cells("f01e14")), "preview focus image", timeout=40)
            frames_before = len(runtime.frames)
            app.send(b"LOCAL-PREVIEW\x1b[200~PASTED-LOCAL\x1b[201~")
            for _ in range(8):
                app.pump()
            assert len(runtime.frames) == frames_before
            app.prefix("c")
            app.wait_text("Stop")
            app.wait(lambda: not app.image_cells("f01e14"), "overlay immediately clears image")
            app.prefix("q")
            app.send(b"\x1b")
            app.wait(lambda: bool(app.image_cells("f01e14")), "modal dismissed without prefix quit", timeout=40)
            app.bind_screen(0)
            app.wait(lambda: not app.image_cells("f01e14"), "Stop removes image")
            app.prefix("2")
            app.bind_screen(0)
            app.wait(lambda: not app.image_cells("141ef0"), "second tab Stop removes image")
            app.wait(lambda: all(not mcp.sessions for mcp in mcps), "MCP sessions normally closed")
            assert all(any(event["method"] == "DELETE" for event in mcp.events) for mcp in mcps)
            passed("Tab-bound previews, focus/paste isolation, modal isolation, Stop clearing and normal MCP cleanup")
            app.prefix("1")
            app.bind_screen(1)
            app.wait(lambda: bool(app.image_cells("f01e14")), "A preview before delayed capture", timeout=40)
            mcps[0].capture_started.clear()
            mcps[0].capture_gate.clear()
            app.wait(mcps[0].capture_started.is_set, "in-flight delayed A capture", timeout=40)
            app.prefix("2")
            app.wait(lambda: not app.image_cells("f01e14"), "switch clears delayed A immediately")
            mcps[0].capture_gate.set()
            for _ in range(25):
                app.pump()
                assert not app.image_cells("f01e14"), "stale A image appeared in second tab"
            app.bind_screen(2)
            app.wait(lambda: bool(app.image_cells("141ef0")), "B preview after delayed A drains", timeout=40)
            mcps[1].image = b"invalid-image"
            app.wait(lambda: not app.image_cells("141ef0") and "桌面暫時不可用" in "".join(app.text().split()), "invalid screenshot removes prior frame", timeout=40)
            mcps[1].image = screenshot_png((20, 30, 240))
            app.wait(lambda: bool(app.image_cells("141ef0")), "valid image recovers after invalid screenshot", timeout=40)
            passed("Delayed stale screenshot cannot cross tabs; invalid image clears and valid capture recovers")

            app.prefix("1")
            app.wait(lambda: bool(app.image_cells("f01e14")), "A binding before revision edit", timeout=40)
            app.prefix("h")
            app.idle()
            app.select_tree("Screen-A")
            edited_screen = next(p for p in tomllib.loads(config.read_text())["desktops"] if p["name"] == "Screen-A")
            app.action("Edit")
            app.wait_text("編輯 Screen")
            app.wait(lambda: not app.image_cells("f01e14"), "edit overlay clears image")
            mcps[0].token = "rotated-synthetic-fixture-token-A"
            app.send(b"\x15")
            app.send("Screen-A-renamed\t\t" + mcps[0].token)
            app.send(b"\x13")
            app.wait(lambda: "Screen-A-renamed" in app.text() and "編輯 Screen" not in app.text(), "edited Screen revision saved")
            app.idle()
            app.wait(lambda: "桌面設定已變更" in "".join(app.text().split()) and not app.image_cells("f01e14"), "revision edit removes old binding")
            edited_profiles = tomllib.loads(config.read_text())["desktops"]
            assert next(p for p in edited_profiles if p["id"] == edited_screen["id"])["name"] == "Screen-A-renamed"
            assert any(p["name"] == "Screen-B" for p in edited_profiles)
            app.prefix("1")
            app.bind_screen(1)
            app.wait(lambda: bool(app.image_cells("f01e14")), "updated Screen token and revision capture", timeout=40)
            app.prefix("h")
            app.idle()
            app.select_tree("Screen-A-renamed")
            app.action("Delete")
            app.wait_text("確認 / Confirm")
            app.send("y")
            app.wait(lambda: len(tomllib.loads(config.read_text()).get("desktops", [])) == 1 and "Screen-A-renamed" not in app.text(), "Screen deletion completed")
            app.wait(lambda: "桌面設定已變更" in "".join(app.text().split()) and not app.image_cells("f01e14"), "deleted Screen binding cleared")
            app.wait_text("Screen-B")
            passed("Screen edit rotates token and invalidates old revision; confirmed deletion clears bindings")
            app.idle()
            app.prefix("1")
            app.prefix("t")
            for _ in range(4):
                app.pump()
            app.resize(14, 60)
            app.wait(lambda: all(state["resizes"] and state["resizes"][-1] == [8, 58] for state in runtime.work().values()), "narrow terminal inner dimensions")
            app.prefix("v")
            app.wait_text("Desktop")
            app.prefix("h")
            app.wait_text("Connections")
            app.prefix("t")
            app.wait_text("Terminal")
            passed("Narrow terminal layout retains keyboard access to all three panes and correct PTY dimensions")
            app.prefix("1")
            app.prefix("t")
            app.send(b"\t\x03\x04")
            app.wait(lambda: b"\t\x03\x04" in b"".join(data for tab, data in runtime.frames if tab == first), "remote Tab/Ctrl-C/Ctrl-D bytes preserved")
            passed("Terminal focus preserves remote Tab, Ctrl-C and Ctrl-D")

            app.resize(42, 132)
            app.wait(lambda: all(state["resizes"] and state["resizes"][-1] == [36, 90] for state in runtime.work().values()), "all-tab resize excludes UI chrome")
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
            app.select_tree(first)
            app.send("\r")
            app.wait_text("確認 / Confirm")
            app.send("y")
            app.wait(lambda: runtime.work()[first]["generation"] == first_generation + 1 and runtime.work()[first]["attached"], "explicit existing Shell attach")
            app.wait_text("2:")
            app.wait_text("Live")
            app.send("REATTACHED-FIRST")
            app.wait(lambda: "REATTACHED-FIRST" in runtime.work()[first]["input"], "reattached Shell input")

            app.prefix("1")
            app.prefix("w")
            app.wait(lambda: not runtime.work()[second]["attached"], "detach second local tab before external takeover fixture")
            runtime.call(runtime.attach_external(second))
            app.wait(lambda: runtime.work()[second]["attached"], "external client attached")
            second_generation = runtime.work()[second]["generation"]
            app.select_tree(second)
            app.send(b"\r")
            app.wait_text("確認 / Confirm")
            app.wait_text("接管")
            app.send("y")
            app.wait(lambda: runtime.work()[second]["generation"] == second_generation + 1 and runtime.work()[second]["attached"], "explicit attached Shell takeover")
            app.wait_text("2:")
            app.wait_text("Live")
            app.send("TAKEN-SECOND")
            app.wait(lambda: "TAKEN-SECOND" in runtime.work()[second]["input"], "takeover input")
            assert len(runtime.work()) == 2
            passed("Existing Shell attach and explicit attached Shell takeover through confirmation UI")

            app.select_tree(first)
            app.action("Delete")
            app.wait_text("確認 / Confirm")
            app.send("y")
            app.wait(lambda: first not in runtime.work(), "explicit remote deletion")
            assert second in runtime.work()
            assert sum(event["method"] == "DELETE" and event["name"] == first for event in runtime.snapshot()["events"]) == 1
            passed("Confirmed remote deletion is separate from local tab detach")
            original_config, original_key = config.read_bytes(), key.read_bytes()
            app.prefix("q")
            app.finish()
            app.save(output, "normal-exit")
            passed("Local tab close preserves work Shell; normal exit restores TTY")

            restarted = Terminal(binary, home, runtime.ca)
            terminals.append(restarted)
            restarted.start()
            restarted.wait_text("PTY fixture")
            assert config.read_bytes() == original_config
            assert key.read_bytes() == original_key
            restarted.process.send_signal(signal.SIGTERM)
            restarted.finish()
            restarted.save(output, "restart-sigterm")
            passed("Prompt-free restart preserves account key and saved profiles; signal exit restores TTY")

            missing_key = key.with_suffix(".held")
            key.rename(missing_key)
            refused = Terminal(binary, home, runtime.ca)
            terminals.append(refused)
            refused.finish(expected=1)
            assert config.read_bytes() == original_config
            assert not key.exists()
            missing_key.rename(key)
            passed("Missing account key fails closed without changing saved profiles")
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
            for mcp in mcps:
                try:
                    mcp.close()
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
