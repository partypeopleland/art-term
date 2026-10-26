#!/usr/bin/env python3
"""Drive the real TUI through the official Runtime and real Shell MCP tools."""
import argparse
import asyncio
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import tomllib


ROOT = Path(__file__).resolve().parents[2]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def dependencies():
    directory = ROOT / "scripts/instance-mcp-regression"
    saved_path = list(sys.path)
    sys.path.insert(0, str(directory))
    saved = {name: sys.modules.get(name) for name in ("fixture", "run")}
    try:
        sys.modules["fixture"] = load("tui_official_mcp_fixture", directory / "fixture.py")
        sys.modules["run"] = load("tui_official_adapter_client", directory / "run.py")
        official = load("tui_official_shell_harness", directory / "official_shell_runtime.py")
        bridge = sys.modules["run"].bridge
        sys.modules["fixture"] = load("tui_official_wire_fixture", ROOT / "scripts/windows-regression/fixture.py")
        driver = load("tui_official_terminal_driver", ROOT / "scripts/tui-regression/run.py")
        return official, bridge, driver
    finally:
        sys.path[:] = saved_path
        for name, original in saved.items():
            if original is None:
                sys.modules.pop(name, None)
            else:
                sys.modules[name] = original


def terminal_cell(app, label, last=False):
    matches = []
    for row in range(app.screen.lines):
        text, columns = [], []
        for column in range(app.screen.columns):
            value = app.screen.buffer[row][column].data
            text.append(value)
            columns.extend([column] * len(value))
        line = "".join(text)
        offset = line.find(label)
        while offset >= 0:
            column = columns[offset]
            width, height = min(app.screen.columns - 4, 88), min(app.screen.lines - 4, 24)
            left, top = (app.screen.columns - width) // 2, (app.screen.lines - height) // 2
            if not last or left < column < left + width - 1 and top < row < top + height - 1:
                matches.append((column, row))
            offset = line.find(label, offset + len(label))
    return (matches[-1] if last else matches[0]) if matches else None


def click(app, label, last=False):
    app.wait(lambda: terminal_cell(app, label, last) is not None, "visible control " + label)
    column, row = terminal_cell(app, label, last)
    app.send(f"\x1b[<0;{column + 1};{row + 1}M\x1b[<0;{column + 1};{row + 1}m")
    for _ in range(4):
        app.pump()


def action(app, label):
    click(app, "[Actions]")
    click(app, label, last=True)


def choose(app, name):
    if terminal_cell(app, "[Desktop]") is not None:
        click(app, "[Desktop]")
        width, height = min(app.screen.columns - 4, 88), min(app.screen.lines - 4, 24)
        left, top = (app.screen.columns - width) // 2, (app.screen.lines - height) // 2
        items = (left + 1, top + 1, left + width - 1, top + height - 4)
        app.wait(lambda: bool(app.locations(name, items)), "selectable Screen row " + name)
        app.click(*app.locations(name, items)[0])
        app.settle()
    else:
        app.prefix("c")
        app.wait_text("Stop")
        app.send(b"\x1b[B" * (1 if name == "Screen A" else 2) + b"\r")


def current_client(bridge, endpoint, method, params):
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(25)
        connection.connect(str(endpoint))
        bridge.send(connection, {"method": method, "params": params})
        return bridge.receive(connection)


def contains_secret(raw, secret):
    if secret in raw or base64.b64encode(secret) in raw:
        return True
    for encoded in re.findall(rb"[A-Za-z0-9+/]{64,}={0,2}", raw):
        for offset in range(4):
            size = (len(encoded) - offset) // 4 * 4
            try:
                if secret in base64.b64decode(encoded[offset:offset + size], validate=True):
                    return True
            except ValueError:
                continue
    return False


class FixedClient:
    def __init__(self, bridge, endpoint):
        self.bridge, self.endpoint = bridge, endpoint

    def call(self, method, params=None):
        return current_client(self.bridge, self.endpoint, method, params or {})

    def tool(self, name, arguments=None):
        return self.call("tools/call", {"name": name, "arguments": arguments or {}})

    def target(self):
        result = self.tool("current_target")
        if result.get("isError"):
            return None
        return json.loads(result["content"][0]["text"])["target"]

    def execute(self, name, arguments=None, target=None):
        return self.tool("call_target_tool", {
            "name": name, "arguments": arguments or {},
            "expected_target": target or self.target(),
        })


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--runtime-binary", type=Path)
    parser.add_argument("--history-bundle", type=Path, help="Compiled shared paged tmux-history.cjs")
    parser.add_argument("--node", type=Path)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    if args.node is None:
        args.node = Path(subprocess.check_output(["node", "-p", "process.execPath"], text=True).strip())
    binary_source = args.binary.resolve(strict=True)
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=False)
    official, bridge, driver = dependencies()
    from aiohttp import ClientSession, web, WSMsgType

    class Gateway(official.TlsRuntimeGateway):
        def __init__(self, root, harness):
            self.controls = []
            self.work_mouse_input = False
            super().__init__(root, harness)

        async def pty(self, request):
            name = request.match_info["name"]
            outbound = web.WebSocketResponse()
            await outbound.prepare(request)
            self.live_ws.add(outbound)
            pending = bytearray()
            binding = None
            async with ClientSession() as client:
                async with client.ws_connect(
                    "http://127.0.0.1:" + str(self.harness.port) + request.raw_path,
                    headers={"Authorization": request.headers.get("Authorization", "")},
                ) as remote:
                    async def forward(source, target, incoming):
                        nonlocal binding
                        async for message in source:
                            if message.type == WSMsgType.BINARY:
                                if incoming and name.startswith("th2-"):
                                    self.work_mouse_input |= b"\x1b[<" in message.data
                                if incoming and name.startswith("mcshot-") and len(pending) < 17000:
                                    pending.extend(message.data)
                                    if binding is None:
                                        found = re.match(rb'exec python3 "\$HOME/\.local/share/art-term/tools/instance_bridge\.py" control --binding (th2-[a-f0-9]{24}) --nonce [a-f0-9]{32}\n', pending)
                                        if found:
                                            binding = found[1].decode()
                                            del pending[:found.end()]
                                    if binding is not None and len(pending) >= 4:
                                        size = int.from_bytes(pending[:4], "big")
                                        if size <= 16384 and len(pending) == size + 4:
                                            payload = json.loads(base64.b64decode(pending[4:], validate=True))
                                            self.controls.append({"shell": name, "binding": binding, "op": payload.get("op")})
                                            payload.clear()
                                            pending.clear()
                                if not incoming:
                                    self.output.append(message.data)
                                await target.send_bytes(message.data)
                            elif message.type == WSMsgType.TEXT:
                                if not incoming:
                                    self.output.append(message.data.encode())
                                await target.send_str(message.data)
                            else:
                                break
                    tasks = [asyncio.create_task(forward(outbound, remote, True)),
                             asyncio.create_task(forward(remote, outbound, False))]
                    try:
                        await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
                    finally:
                        for task in tasks:
                            task.cancel()
                        await asyncio.gather(*tasks, return_exceptions=True)
                        pending.clear()
            self.live_ws.discard(outbound)
            await outbound.close()
            return outbound

    checks, apps, services, bindings = [], [], [], []
    harness = gateway = app = None

    def passed(case, message):
        checks.append({"case": case, "result": "PASS", "check": message})
        print("PASS " + case + " " + message, flush=True)

    def wait(predicate, message, timeout=30):
        app.wait(predicate, message, timeout)

    with tempfile.TemporaryDirectory(prefix="art-term-m-") as directory:
        root = Path(directory)
        binary = root / "art-term"
        shutil.copy2(binary_source, binary)
        binary.chmod(0o700)
        try:
            harness = official.linux.Harness(root, args.runtime_binary)
            tools = harness.home / ".local/share/art-term/tools"
            tools.mkdir(parents=True, mode=0o700)
            for helper in ("instance_bridge.py", "instance-shell.py", "instance-shell.bash"):
                shutil.copy2(ROOT / "scripts" / helper, tools / helper)
            shutil.copy2(ROOT / "scripts/openab-runtime/tmux-v2/tmux-shell.py", tools / "tmux-shell.py")
            tmux = harness.helper / "tmux-v2"
            tmux.mkdir()
            shutil.copy2(shutil.which("tmux"), tmux / "tmux")
            (tmux / "tmux").chmod(0o700)
            if args.history_bundle is None:
                subprocess.run([str(args.node), str(ROOT / "scripts/openab-runtime/build-tmux-history.mjs"),
                                "--paged", str(tmux)], check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            else:
                shutil.copy2(args.history_bundle, tmux / "tmux-history.cjs")
            shutil.copy2(args.node.resolve(strict=True), harness.helper / "node")
            (harness.helper / "node").chmod(0o700)
            (harness.home / ".bashrc").write_text("export PS1='ARTTERM_TUI_READY> '\n")
            proxy = root / "proxy"
            proxy.mkdir(mode=0o700)
            gateway = Gateway(proxy, harness)
            a = official.McpFixture("A", protocol="2024-11-05")
            b = official.McpFixture("B", sse=True)
            a.image = driver.screenshot_png((240, 30, 20))
            b.image = driver.screenshot_png((20, 30, 240))
            services.extend([a, b])
            home = root / "client"
            home.mkdir(mode=0o700)
            app = driver.Terminal(binary, home, gateway.ca, rows=44, columns=150)
            apps.append(app)
            app.start()
            modern = terminal_cell(app, "[Actions]") is not None
            if modern:
                action(app, "New Host")
            else:
                app.send("n")
            app.wait_text("HTTPS Runtime URL")
            app.send("Official Runtime\t" + gateway.origin + "\t" + harness.credential)
            app.send(b"\x13")
            wait(lambda: "Official Runtime" in app.text() and "HTTPS Runtime URL" not in app.text(), "saved Runtime Host")
            app.idle()
            for index, service in enumerate(services):
                if modern:
                    action(app, "New Screen")
                else:
                    app.prefix("h")
                    app.idle()
                    app.send("g")
                    app.send("n")
                app.wait_text("新增 Screen")
                app.send("Screen " + service.name + "\t" + service.url + "\t" + service.token)
                app.send(b"\x13")
                wait(lambda: "新增 Screen" not in app.text(), "saved Screen " + service.name)
                app.idle()

            def works():
                return [item["name"] for item in harness.api("GET", "/admin/sessions")[1]["sessions"]
                        if item["name"].startswith("th2-")]

            def create_work():
                previous = set(works())
                if modern:
                    click(app, "+ New Shell")
                else:
                    app.prefix("n")
                wait(lambda: len(set(works()) - previous) == 1, "real Runtime work Shell creation")
                name = next(iter(set(works()) - previous))
                bindings.append(name)
                app.idle()
                app.wait_text("ARTTERM_TUI_READY>")
                return name

            def start_client(binding, number):
                endpoint = root / f"client-{number}.sock"
                app.prefix("t")
                app.send(f"python3 {ROOT}/scripts/instance-mcp-regression/remote_client.py --binding {binding} --socket {endpoint}\r")
                wait(endpoint.exists, "fixed client started inside App work Shell")
                client = FixedClient(bridge, endpoint)
                assert {item["name"] for item in client.call("tools/list")["tools"]} == {
                    "current_target", "list_target_tools", "call_target_tool"}
                return client

            first = create_work()
            a.list_error = True
            choose(app, "Screen A")
            wait(lambda: bool(app.image_cells("f01e14")) and "Tools unconfirmed" in app.text(),
                 "A valid screenshot remains visible when tools/list fails")
            assert "Tools ready" not in app.text()
            passed("T-SCREEN/T-TOOLS", "A valid preview remains available when the real upstream tools/list fails; tools are unconfirmed")
            a.list_error = False
            valid_a_image = a.image
            a.image = b"desktop-not-started-invalid-image"
            choose(app, "Screen A")
            wait(lambda: "Desktop: Screen A | Tools ready" in app.text(),
                 "MCP tools must configure before the virtual desktop can produce a screenshot")
            one = start_client(first, 1)
            wait(lambda: one.target() is not None,
                 "BUG: selected Screen preview exists but the real Shell stdio client has no configured target", 15)
            target_a = one.target()
            result = one.execute("browser_navigate", {"url": "https://fixture.example/tui-A"})
            assert not result.get("isError") and '"desktop": "A"' in result["content"][0]["text"]
            assert "tui-A" in result["content"][0]["text"]
            assert any(item["binding"] == first and item["op"] == "configure" for item in gateway.controls)
            assert "Tools ready" in app.text()
            assert not app.image_cells("f01e14"), "Unavailable screenshot must not fabricate an image"
            passed("T-CONFIGURE/T-TOOLS", "Tools configure and execute before a virtual desktop screenshot is available")
            a.image = valid_a_image
            wait(lambda: bool(app.image_cells("f01e14")), "actual A screenshot recovers after desktop starts")
            app.save(output, "selected-A-tools-ready")
            passed("T-CONFIGURE/T-TOOLS", "TUI selection configures the exact work Shell; a late real stdio client invokes HTTP desktop A")

            second = create_work()
            two = start_client(second, 2)
            assert two.target() is None, "new work Shell inherited another Shell's target"
            choose(app, "Screen B")
            wait(lambda: two.target() is not None, "second work Shell configured")
            target_b = two.target()
            result = two.execute("browser_navigate", {"url": "https://fixture.example/tui-B"})
            assert not result.get("isError") and '"desktop": "B"' in result["content"][0]["text"]
            assert one.target() == target_a
            assert '"desktop": "A"' in one.execute("browser_title")["content"][0]["text"]
            passed("T-TARGET", "Two real App work Shells keep independent broker selections and tool routes")

            app.prefix("1")
            a.pause = True
            in_flight = {}
            thread = threading.Thread(target=lambda: in_flight.update(result=one.execute(
                "browser_navigate", {"url": "https://fixture.example/in-flight-A"}, target_a)))
            thread.start()
            assert a.started.wait(3), "real A tool request never began"
            try:
                choose(app, "Screen B")
                wait(lambda: "Desktop: Screen B | Tools ready" in app.text(), "A to B selection")
            finally:
                a.release.set()
                thread.join(timeout=10)
            assert not thread.is_alive()
            assert in_flight["result"]["_meta"]["artTermTarget"] == target_a
            assert "in-flight-A" in in_flight["result"]["content"][0]["text"]
            before = len(b.events)
            assert one.execute("browser_navigate", {"url": "https://wrong.fixture/"}, target_a)["isError"]
            assert len(b.events) == before, "stale expected_target reached B"
            assert '"desktop": "B"' in one.execute("browser_title")["content"][0]["text"]
            passed("T-TARGET", "A call remains on A during a TUI switch; stale target is rejected before B")

            selected = one.target()
            before = len(gateway.controls)
            click(app, "[Desktop]")
            click(app, "Stop")
            wait(lambda: not app.image_cells("141ef0"), "Stop clears local preview")
            assert one.target() == selected
            assert not any(item["op"] == "clear" for item in gateway.controls[before:])
            assert not one.execute("browser_title").get("isError")
            passed("T-LIFETIME", "Stopping preview leaves the real Shell tools selected and usable")

            app.prefix("w")
            wait(lambda: not next(item for item in harness.api("GET", "/admin/sessions")[1]["sessions"]
                                  if item["name"] == first)["attached"], "local work WebSocket detached")
            assert one.target() == selected
            before = len(gateway.controls)
            app.prefix("h")
            app.idle()
            click(app, "Refresh Shells")
            wait(lambda: any(first in line and "●" not in line for line in app.text().splitlines()),
                 "refreshed detached Shell row")
            click(app, first)
            click(app, "[Yes]")
            wait(lambda: any(item["binding"] == first and item["op"] == "verify"
                             for item in gateway.controls[before:]), "reattach verifies existing broker")
            wait(lambda: "Tools ready" in app.text(), "reattached Shell restores tool state")
            assert one.target() == selected, "restore changed broker target or generation"
            assert not any(item["op"] in ("configure", "clear") for item in gateway.controls[before:])
            assert any(item["binding"] == first and item["op"] == "verify" for item in gateway.controls[before:])
            passed("T-RESTORE", "Reattach restores through current/verify without reconfiguring or changing generation")

            app.prefix("q")
            app.finish()
            assert one.target() == selected and two.target() == target_b
            app.save(output, "quit-preserves-tools")
            app = driver.Terminal(binary, home, gateway.ca, rows=44, columns=150)
            apps.append(app)
            app.start()
            before = len(gateway.controls)
            click(app, first)
            click(app, "[Yes]")
            wait(lambda: any(item["binding"] == first and item["op"] == "verify"
                             for item in gateway.controls[before:]), "restart verifies existing broker")
            wait(lambda: "Tools ready" in app.text(), "restart restores existing Shell desktop")
            assert one.target() == selected and two.target() == target_b
            assert not any(item["op"] in ("configure", "clear") for item in gateway.controls[before:])
            passed("T-RESTORE/T-LIFETIME", "TUI quit/restart preserves Shell-owned selections and restores the original generation")

            app.prefix("q")
            app.finish()
            before_captures = sum(event.get("method") == "tools/call" and event.get("params", {}).get("name") == "screenshot" for service in services for event in service.events)
            app = driver.Terminal(binary, home, gateway.ca, rows=44, columns=150,
                                  env_overrides={"SSH_CONNECTION": "fixture-ssh-session"})
            apps.append(app)
            app.start()
            wait(lambda: "SSH" in app.text() and "桌面工具仍可使用" in "".join(app.text().replace("│", "").split()), "SSH local-only preview notice")
            click(app, first)
            click(app, "[Yes]")
            wait(lambda: "Tools ready" in app.text(), "SSH reattach restores desktop tools without preview")
            before = len(gateway.controls)
            choose(app, "Screen A")
            wait(lambda: "Desktop: Screen A | Tools ready" in app.text(), "SSH Screen setup without preview")
            ssh_target = one.target()
            assert ssh_target is not None and ssh_target != selected
            listed = one.tool("list_target_tools")
            assert not listed.get("isError"), listed
            assert "browser_navigate" in json.dumps(listed)
            result = one.execute("browser_navigate", {"url": "https://fixture.example/ssh-no-preview"}, ssh_target)
            assert not result.get("isError") and '"desktop": "A"' in result["content"][0]["text"]
            assert "ssh-no-preview" in result["content"][0]["text"]
            assert any(item["binding"] == first and item["op"] == "configure" for item in gateway.controls[before:])
            app.settle(.4)
            assert not app.sixels, "SSH preview emitted native image"
            assert sum(event.get("method") == "tools/call" and event.get("params", {}).get("name") == "screenshot" for service in services for event in service.events) == before_captures, "SSH preview requested screenshot"
            assert two.target() == target_b, "SSH selection affected another Shell"
            passed("T-CONFIGURE/T-TOOLS/T-IMAGE-POLICY", "SSH disables only local preview; actual Shell stdio discovers target/tools and invokes desktop A without screenshot")

            click(app, "[Desktop]")
            click(app, "Clear tools")
            click(app, "[Yes]")
            wait(lambda: one.target() is None, "explicit clear rejects new Shell tool calls")
            assert one.execute("browser_title")["isError"]
            assert two.target() == target_b and not two.execute("browser_title").get("isError")
            passed("T-LIFETIME", "Explicit clear affects only the selected work Shell and makes its new calls fail")

            control_shells = {item["shell"] for item in gateway.controls}
            wait(lambda: not control_shells.intersection(
                item["name"] for item in harness.api("GET", "/admin/sessions")[1]["sessions"]),
                "owned temporary configure Shell cleanup")
            observed = b"".join(gateway.output + gateway.replay)
            assert not gateway.work_mouse_input, "local mouse controls reached the work PTY"
            for service in services:
                secret = service.token.encode()
                assert not contains_secret(observed, secret) and not contains_secret(app.raw, secret)
                for history in harness.home.rglob("*"):
                    if history.is_file():
                        raw = history.read_bytes()
                        assert secret not in raw, "upstream token persisted on Runtime Host"
                        if len(raw) <= 1024 * 1024:
                            assert not contains_secret(raw, secret), "encoded upstream token persisted on Runtime Host"
                for proc in Path("/proc").glob("[0-9]*"):
                    try:
                        command = (proc / "cmdline").read_bytes()
                        if any(name.encode() in command for name in bindings) and (
                            b"instance_bridge.py" in command or b"remote_client.py" in command
                        ):
                            assert secret not in command + (proc / "environ").read_bytes(), "upstream token in argv/env"
                    except (PermissionError, ProcessLookupError, FileNotFoundError):
                        continue
            passed("T-SECRET", "No upstream token in work output, Host files or tool argv/env; temporary control Shells are deleted")

            app.prefix("q")
            app.finish()
            broker_paths = [Path("/tmp") / ("art-term-" + str(os.getuid())) /
                            hashlib.sha256(name.encode()).hexdigest()[:24] / "shell.sock" for name in bindings]
            for name in list(works()):
                status, _ = harness.api("DELETE", "/admin/sessions/" + name)
                assert status == 200
            deadline = time.monotonic() + 8
            while any(path.exists() for path in broker_paths) and time.monotonic() < deadline:
                time.sleep(.05)
            assert all(not path.exists() for path in broker_paths), "Runtime Shell delete leaked its broker"
            passed("T-LIFETIME", "Deleting the real Runtime work Shell tears down its Shell broker")
            result = {"result": "PASS", "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                      "runtime_sha256": official.linux.artifact.BINARY_SHA256, "checks": checks,
                      "real_ai_model": "NOT_RUN", "owner_endpoints": "NOT_RUN"}
            (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
            print("All official Runtime TUI tool checks passed.", flush=True)
        finally:
            errors = []
            for client in apps:
                try:
                    client.save(output, "terminal-" + str(apps.index(client)))
                    client.close()
                except Exception as error:
                    errors.append(error)
            for service in services:
                service.release.set()
                service.list_release.set()
            if harness is not None:
                try:
                    if getattr(harness, "ready", False):
                        harness.owned.update(item["name"] for item in harness.api("GET", "/admin/sessions")[1]["sessions"])
                    harness.close()
                except Exception as error:
                    errors.append(error)
            if gateway is not None:
                try:
                    gateway.close()
                except Exception as error:
                    errors.append(error)
                (output / "control-operations.json").write_text(json.dumps(gateway.controls, indent=2) + "\n")
            for service in services:
                try:
                    service.close()
                except Exception as error:
                    errors.append(error)
            for name in bindings:
                path = Path("/tmp") / ("art-term-" + str(os.getuid())) / hashlib.sha256(name.encode()).hexdigest()[:24]
                if path.exists() and not (path / "shell.sock").exists():
                    shutil.rmtree(path)
            if errors:
                raise ExceptionGroup("Official Runtime TUI fixture cleanup failed", errors)


if __name__ == "__main__":
    main()
