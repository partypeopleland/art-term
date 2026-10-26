#!/usr/bin/env python3
"""Inspect live art-term Sixel pixels painted by an isolated xterm/Xvfb client."""
import argparse
import codecs
import ctypes
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import pty
import select
import signal
import io
import socket
import struct
import subprocess
import sys
import tempfile
import termios
import time
import traceback
import tty


LIBC = ctypes.CDLL(None, use_errno=True)
LIBC.pidfd_open.argtypes = [ctypes.c_int, ctypes.c_uint]
LIBC.pidfd_open.restype = ctypes.c_int
LIBC.pidfd_send_signal.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p, ctypes.c_uint]
LIBC.pidfd_send_signal.restype = ctypes.c_int


def open_pidfd(pid):
    descriptor = LIBC.pidfd_open(pid, 0)
    if descriptor < 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))
    return descriptor


def signal_pidfd(descriptor, signum):
    if LIBC.pidfd_send_signal(descriptor, signum, None, 0) < 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))


def process_start(pid):
    return Path(f"/proc/{pid}/stat").read_text().split(") ", 1)[1].split()[19]


def stop_child(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=2)


def proxy(binary, control, recording, metadata):
    original = termios.tcgetattr(0)
    master, slave = pty.openpty()
    dimensions = fcntl.ioctl(0, termios.TIOCGWINSZ, b"\0" * 8)
    fcntl.ioctl(slave, termios.TIOCSWINSZ, dimensions)
    Path(metadata).write_text(json.dumps({"winsize": struct.unpack("HHHH", dimensions),
        "TERM": os.environ.get("TERM"), "XTERM_VERSION": os.environ.get("XTERM_VERSION")}))
    server = socket.socket(socket.AF_UNIX)
    server.bind(control)
    server.listen(1)
    process = None
    def interrupted(signum, _frame):
        raise SystemExit(128 + signum)
    for signum in (signal.SIGHUP, signal.SIGTERM, signal.SIGINT):
        signal.signal(signum, interrupted)
    try:
        tty.setraw(0)
        def controlling_terminal():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)
        process = subprocess.Popen([binary], stdin=slave, stdout=slave, stderr=slave,
                                   preexec_fn=controlling_terminal, close_fds=True)
        os.close(slave)
        values = json.loads(Path(metadata).read_text())
        values.update(app_pid=process.pid, app_start=process_start(process.pid), proxy_pid=os.getpid())
        Path(metadata).write_text(json.dumps(values))
        client, _ = server.accept()
        with client, open(recording, "wb", buffering=0) as record:
            while True:
                ready, _, _ = select.select([master, client], [], [], .1)
                if master in ready:
                    try:
                        data = os.read(master, 65536)
                    except OSError:
                        break
                    if not data:
                        break
                    record.write(data)
                    os.write(1, data)
                if client in ready:
                    data = client.recv(65536)
                    if not data:
                        stop_child(process)
                        break
                    os.write(master, data)
                if process.poll() is not None and not ready:
                    break
        code = process.wait(timeout=10)
        values = json.loads(Path(metadata).read_text())
        values["app_exit_code"] = code
        Path(metadata).write_text(json.dumps(values))
        return code
    finally:
        for signum in (signal.SIGHUP, signal.SIGTERM, signal.SIGINT):
            signal.signal(signum, signal.SIG_IGN)
        if process:
            stop_child(process)
        try:
            termios.tcsetattr(0, termios.TCSANOW, original)
        except termios.error:
            pass
        server.close()
        os.close(master)


def load_driver():
    spec = importlib.util.spec_from_file_location("native_client_driver", Path(__file__).with_name("run.py"))
    driver = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(driver)
    return driver


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--xterm", type=Path, required=True)
    parser.add_argument("--xvfb", type=Path, default=Path("/usr/bin/Xvfb"))
    parser.add_argument("--library-path")
    parser.add_argument("--explicit", action="store_true", help="Baseline explicit Sixel configuration")
    args = parser.parse_args()
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=False)
    binary = args.binary.resolve(strict=True)
    result = {"binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "checks": [],
              "mode": "live-xterm-rendered-pixels", "physical_os_ime": "NOT_RUN"}
    driver = load_driver()
    from PIL import Image
    from Xlib import display, X
    runtime = mcp = bootstrap = terminal = xvfb = None
    xdisplay = process = channel = None
    temporary = tempfile.TemporaryDirectory(prefix="art-term-native-client-")
    try:
        directory = temporary.name
        root = Path(directory)
        home, fixtures = root / "home", root / "fixtures"
        home.mkdir(mode=0o700)
        fixtures.mkdir(mode=0o700)
        runtime = driver.Runtime(fixtures)
        mcp = driver.McpFixture("Native pixels")
        source = Image.new("RGB", (640, 320))
        for y in range(320):
            for x in range(640):
                color = (240, 30, 20) if (x < 320) == (y < 160) else (30, 220, 60)
                if 200 <= x < 440 and 120 <= y < 200:
                    color = (250, 0, 250) if (x // 4) % 2 else (0, 250, 250)
                source.putpixel((x, y), color)
        encoded = io.BytesIO()
        source.save(encoded, format="PNG")
        mcp.image = encoded.getvalue()
        bootstrap = driver.Terminal(binary, home, runtime.ca)
        bootstrap.start()
        bootstrap.click_text("New Host")
        bootstrap.wait_text("HTTPS Runtime URL")
        bootstrap.send("Native pixels\t" + runtime.origin + "\t" + runtime.credential)
        bootstrap.send(b"\x13")
        bootstrap.wait(lambda: "HTTPS Runtime URL" not in bootstrap.text(), "Host saved")
        bootstrap.idle()
        bootstrap.action("New Screen")
        bootstrap.wait_text("新增 Screen")
        bootstrap.send("Native pixels\t" + mcp.url + "\t" + mcp.token)
        bootstrap.send(b"\x13")
        bootstrap.wait(lambda: "新增 Screen" not in bootstrap.text(), "Screen saved")
        bootstrap.idle()
        bootstrap.prefix("q")
        bootstrap.finish()
        bootstrap.close()
        bootstrap = None
        config = home / "config/art-term-tui/display.toml"
        if args.explicit:
            config.write_text("image_mode='sixel'\n")
            config.chmod(0o600)
        result["display_config"] = config.read_text()
        before = config.read_bytes()
        read_fd, write_fd = os.pipe()
        xvfb_log = (output / "xvfb.log").open("wb")
        xvfb = subprocess.Popen([str(args.xvfb), "-displayfd", str(write_fd), "-screen", "0", "1600x1200x24", "-nolisten", "tcp", "-ac"],
                                pass_fds=(write_fd,), stdout=xvfb_log, stderr=xvfb_log)
        os.close(write_fd)
        with os.fdopen(read_fd) as channel:
            number = channel.readline().strip()
        assert number.isdigit(), "Xvfb did not allocate a display"
        env = os.environ.copy()
        for key in ("SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY", "TMUX", "STY", "DISPLAY", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS", "DBUS_SYSTEM_BUS_ADDRESS", "XTERM_VERSION", "TERM"):
            env.pop(key, None)
        env.update(HOME=str(home), XDG_CONFIG_HOME=str(home / "config"), XDG_DATA_HOME=str(home / "data"), XDG_CACHE_HOME=str(home / "cache"),
                   DISPLAY=":" + number, SHELL="/bin/sh", COLORTERM="truecolor", LC_ALL="C.UTF-8", ART_TERM_OPENAB_FIXTURE_CA=str(runtime.ca))
        if args.library_path:
            env["LD_LIBRARY_PATH"] = args.library_path
        recording, metadata, control = root / "terminal.ansi", output / "terminal.json", root / "input.sock"
        xterm_log = (output / "xterm.log").open("wb")
        command = [str(args.xterm.resolve()), "-ti", "vt340", "-tn", "vt340", "-geometry", "120x36+0+0", "-b", "0", "-fn", "fixed", "-title", "ArtTermNativeVerify", "-e",
                   sys.executable, str(Path(__file__).resolve()), "--proxy", str(binary), str(control), str(recording), str(metadata)]
        result["xterm_arguments"] = command[:command.index("-e")]
        process = subprocess.Popen(command, env=env, stdout=xterm_log, stderr=xterm_log, start_new_session=True)

        def close_client():
            app_fd = None
            try:
                values = json.loads(metadata.read_text())
                app_fd = open_pidfd(values["app_pid"])
                if process_start(values["app_pid"]) != values["app_start"]:
                    os.close(app_fd)
                    app_fd = None
            except (OSError, KeyError, ValueError):
                if app_fd is not None:
                    os.close(app_fd)
                    app_fd = None
            if channel:
                channel.close()
            try:
                if process.poll() is None:
                    try:
                        process.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        stop_child(process)
                if app_fd is not None and not select.select([app_fd], [], [], 0)[0]:
                    try:
                        signal_pidfd(app_fd, signal.SIGTERM)
                        if not select.select([app_fd], [], [], 2)[0]:
                            signal_pidfd(app_fd, signal.SIGKILL)
                            assert select.select([app_fd], [], [], 2)[0], "Owned TUI child did not exit"
                    except ProcessLookupError:
                        pass
            finally:
                if app_fd is not None:
                    os.close(app_fd)
        deadline = time.monotonic() + 10
        while not control.exists() and time.monotonic() < deadline:
            assert process.poll() is None, (output / "xterm.log").read_text()
            time.sleep(.03)
        assert control.exists(), "xterm child did not create its control socket"
        channel = socket.socket(socket.AF_UNIX)
        channel.connect(str(control))
        xdisplay = display.Display(env["DISPLAY"])
        xroot = xdisplay.screen().root

        class LiveTerminal(driver.Terminal):
            def __init__(self):
                self.screen = driver.Screen(120, 36)
                self.stream = driver.pyte.Stream(self.screen)
                self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
                self.raw, self.pending, self.sixels = bytearray(), bytearray(), []
                self.process, self.offset = process, 0
            def send(self, data):
                channel.sendall(data.encode() if isinstance(data, str) else data)
            def pump(self, timeout=.05):
                time.sleep(timeout)
                if not recording.exists():
                    return
                data = recording.read_bytes()[self.offset:]
                self.offset += len(data)
                self.raw.extend(data)
                self.pending.extend(data)
                while self.pending:
                    start = self.pending.find(b"\x1bP")
                    if start < 0:
                        size = len(self.pending) - int(self.pending.endswith(b"\x1b"))
                        self.stream.feed(self.decoder.decode(bytes(self.pending[:size])))
                        del self.pending[:size]
                        return
                    if start:
                        self.stream.feed(self.decoder.decode(bytes(self.pending[:start])))
                        del self.pending[:start]
                    end = self.pending.find(b"\x1b\\", 2)
                    if end < 0:
                        return
                    self.sixels.append(bytes(self.pending[:end + 2]))
                    del self.pending[:end + 2]
            def close(self):
                close_client()
            def finish(self, expected=0):
                self.wait(lambda: process.poll() is not None, "xterm child exits")
                assert process.returncode == expected, process.returncode
        terminal = LiveTerminal()
        terminal.start()
        result["terminal"] = json.loads(metadata.read_text())
        rows, columns, width, height = result["terminal"]["winsize"]
        assert (rows, columns) == (36, 120)
        assert width and height and width % columns == 0 and height % rows == 0
        result["cell_pixels"] = [width // columns, height // rows]
        assert result["terminal"]["TERM"] == "vt340"
        assert result["terminal"]["XTERM_VERSION"] == "XTerm(390)"

        def capture(name):
            xdisplay.sync()
            windows = xroot.query_tree().children
            window = next(window for window in windows if window.get_wm_name() == "ArtTermNativeVerify")
            geometry = window.get_geometry()
            data = window.get_image(0, 0, geometry.width, geometry.height, X.ZPixmap, 0xffffffff)
            image = Image.frombytes("RGB", (geometry.width, geometry.height), data.data, "raw", "BGRX")
            image.save(output / (name + ".png"))
            return image

        def colored(image, right=False):
            region = image.crop((image.width // 3, 0, image.width, image.height)) if right else image
            return sum(1 for color in region.get_flattened_data() if color[0] > 180 and color[1] < 80 and color[2] < 80)

        terminal.click_text("New Shell", (0, 1, 40, 24))
        terminal.wait(lambda: len(runtime.work()) == 1, "work created")
        terminal.wait_text("Live")
        terminal.bind_screen(1)
        terminal.wait(lambda: bool(terminal.sixels), "actual Sixel thumbnail")
        terminal.settle(.3)
        thumbnail = capture("thumbnail")
        assert colored(thumbnail) > 500, "Real xterm did not paint the known red thumbnail pixels"
        assert colored(thumbnail, True) == 0, "Thumbnail leaked into the right work pane"
        result["thumbnail_red_pixels"] = colored(thumbnail)
        terminal.click(15, 27)
        terminal.wait_text("[X]")
        terminal.wait(lambda: len(terminal.sixels) >= 3, "expanded native raster")
        terminal.settle(.3)
        expanded = capture("expanded")
        assert colored(expanded, True) > 2000, "Real xterm did not paint the right expanded native image"
        result["expanded_right_red_pixels"] = colored(expanded, True)
        cell_width = result["cell_pixels"][0]
        subcell_rows = 0
        for y in range(expanded.height):
            for x in range((40 * cell_width), expanded.width - cell_width, cell_width):
                colors = [expanded.getpixel((column, y)) for column in range(x, x + cell_width)]
                pink = any(r > 140 and b > 140 and g < 100 for r, g, b in colors)
                cyan = any(g > 140 and b > 140 and r < 100 for r, g, b in colors)
                subcell_rows += pink and cyan
        assert subcell_rows > 20, "Real image detail did not vary horizontally inside character cells"
        result["within_cell_native_color_rows"] = subcell_rows
        terminal.click_text("[X]", (40, 1, 120, 4))
        terminal.wait(lambda: "[X]" not in terminal.text(), "viewer closed")
        terminal.settle(.3)
        closed = capture("closed")
        assert colored(closed, True) == 0, "Closing the viewer left real native pixels in the work pane"
        assert colored(closed) > 500, "Closing the viewer lost the left thumbnail"
        terminal.prefix("c")
        terminal.wait_text("Stop")
        terminal.settle(.3)
        modal = capture("modal")
        assert colored(modal) == 0, "Native image painted through the real modal"
        terminal.send(b"\x1b")
        terminal.wait(lambda: "Stop" not in terminal.text(), "modal closed")
        terminal.settle(.3)
        terminal.prefix("q")
        terminal.finish()
        result["terminal"] = json.loads(metadata.read_text())
        assert result["terminal"]["app_exit_code"] == 0, result["terminal"]
        assert config.read_bytes() == before, "Existing display settings were rewritten"
        (output / "terminal.ansi").write_bytes(terminal.raw)
        (output / "terminal.txt").write_text(terminal.text())
        result["checks"].append({"id": "T-IMAGE-CLIENT", "result": "PASS", "description": "Live TUI paints original native Sixel pixels in real xterm thumbnail and right viewer; X/modal clear the actual painted pixels"})
        result["actual_sixel_client"] = "PASS"
        result["result"] = "PASS"
    except Exception:
        result["result"] = "FAIL"
        result["failure"] = traceback.format_exc()
        if terminal:
            (output / "failure.ansi").write_bytes(terminal.raw)
            (output / "failure.txt").write_text(terminal.text())
        print(result["failure"], flush=True)
    finally:
        if terminal:
            terminal.close()
        if bootstrap:
            bootstrap.close()
        if not terminal and process:
            close_client()
        elif not terminal and channel:
            channel.close()
        if process and process.poll() is None:
            stop_child(process)
        if xdisplay:
            xdisplay.close()
        if xvfb:
            xvfb.terminate()
            xvfb.wait(timeout=10)
        if mcp:
            mcp.capture_gate.set()
            mcp.close()
        if runtime:
            runtime.close()
        temporary.cleanup()
        (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    if result.get("result") != "PASS":
        raise SystemExit(1)
    print("PASS live local xterm native pixel rendering and clear", flush=True)


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--proxy":
        raise SystemExit(proxy(*sys.argv[2:]))
    main()
