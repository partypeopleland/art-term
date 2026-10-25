#!/usr/bin/env python3
import argparse
import base64
import hashlib
import json
import os
import pathlib
import secrets
import select
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

parser = argparse.ArgumentParser(
    description="Verify the native Linux desktop in a private Xvfb and Secret Service session."
)
parser.add_argument(
    "--binary",
    type=pathlib.Path,
    required=True,
    help="Built or extracted package executable",
)
parser.add_argument(
    "--output-dir",
    type=pathlib.Path,
    required=True,
    help="New directory for logs, screenshots and result.json",
)
parser.add_argument(
    "--application-id",
    default="net.missart.artterm.desktop",
    help="Compiled Tauri application identifier",
)
parser.add_argument(
    "--disable-webkit-sandbox",
    action="store_true",
    help="Only for isolated test containers that cannot run WebKit's sandbox",
)
args = parser.parse_args()
BINARY = str(args.binary.resolve(strict=True))
if not os.path.isfile(BINARY) or not os.access(BINARY, os.X_OK):
    parser.error("--binary must be an executable file")
if (
    not args.application_id
    or "/" in args.application_id
    or args.application_id in (".", "..")
):
    parser.error("invalid application identifier")
for dependency in [
    "dbus-daemon",
    "gnome-keyring-daemon",
    "gdbus",
    "Xvfb",
    "xauth",
    "xdotool",
    "WebKitWebDriver",
]:
    if shutil.which(dependency) is None:
        parser.error("missing dependency: " + dependency)
output = args.output_dir.resolve()
output.mkdir(parents=True, exist_ok=False)
base = pathlib.Path(tempfile.mkdtemp(prefix="art-term-linux-client-"))
processes = []
session = None
home = base / "home"
home.mkdir(mode=0o700)
run = base / "run"
run.mkdir(mode=0o700)
env = os.environ.copy()
for inherited in [
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "DBUS_STARTER_ADDRESS",
    "DBUS_STARTER_BUS_TYPE",
    "SSH_AUTH_SOCK",
    "GPG_AGENT_INFO",
    "WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS",
]:
    env.pop(inherited, None)
env.update(
    HOME=str(home),
    XDG_CONFIG_HOME=str(home / "config"),
    XDG_DATA_HOME=str(home / "data"),
    XDG_CACHE_HOME=str(home / "cache"),
    XDG_RUNTIME_DIR=str(run),
    DBUS_SESSION_BUS_ADDRESS="unix:path=" + str(base / "bus"),
    GNOME_KEYRING_CONTROL=str(run / "keyring"),
    DBUS_SYSTEM_BUS_ADDRESS="unix:path=" + str(base / "no-system-bus"),
    TAURI_WEBVIEW_AUTOMATION="true",
    LIBGL_ALWAYS_SOFTWARE="1",
)
if args.disable_webkit_sandbox:
    env["WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS"] = "1"
with socket.socket() as reservation:
    reservation.bind(("127.0.0.1", 0))
    port = reservation.getsockname()[1]
URL = "http://127.0.0.1:" + str(port)
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def spawn(command, name, stdin=None, pass_fds=()):
    with open(output / (name + ".log"), "wb") as log:
        p = subprocess.Popen(
            command,
            env=env,
            stdin=subprocess.PIPE if stdin is not None else subprocess.DEVNULL,
            stdout=log,
            stderr=log,
            start_new_session=True,
            pass_fds=pass_fds,
        )
    processes.append(p)
    if stdin is not None:
        p.stdin.write(stdin)
        p.stdin.close()
    return p


def request(method, path, data=None, timeout=30):
    r = urllib.request.Request(
        URL + path,
        data=None if data is None else json.dumps(data).encode(),
        method=method,
        headers={"Content-Type": "application/json"},
    )
    try:
        with opener.open(r, timeout=timeout) as response:
            result = json.load(response)
    except urllib.error.HTTPError as e:
        raise RuntimeError(e.read().decode())
    return result.get("value")


def wd(method, path, data=None):
    return request(method, "/session/" + session + path, data)


def js(script, *args):
    return wd("POST", "/execute/sync", {"script": script, "args": list(args)})


def wait(predicate, message, timeout=20):
    stop = time.monotonic() + timeout
    last = None
    while time.monotonic() < stop:
        try:
            value = predicate()
            if value:
                return value
        except (OSError, RuntimeError, ValueError, KeyError) as e:
            last = e
        time.sleep(0.1)
    raise RuntimeError(message + " timeout; last=" + str(last))


def selector(css):
    return js("return !!document.querySelector(arguments[0]);", css)


def element(css):
    return wd("POST", "/element", {"using": "css selector", "value": css})[
        "element-6066-11e4-a52e-4f735466cecf"
    ]


def click(css):
    wait(lambda: selector(css), "find " + css)
    wait(
        lambda: js(
            "const el = document.querySelector(arguments[0]);el.scrollIntoView({block:'center',inline:'center',behavior:'instant'});return !el.disabled && el.getClientRects().length > 0;",
            css,
        ),
        "interactable " + css,
    )
    wd("POST", "/element/" + element(css) + "/click", {})


def text():
    return js("return document.body.innerText")


def fill(css, value):
    el = element(css)
    wd("POST", "/element/" + el + "/clear", {})
    wd("POST", "/element/" + el + "/value", {"text": value})


def invoke(command, args=None):
    return wd(
        "POST",
        "/execute/async",
        {
            "script": "const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke(arguments[0],arguments[1]).then(value=>done({ok:true,value}),error=>done({ok:false,error}));",
            "args": [command, args or {}],
        },
    )


def shot(name):
    b = wd("GET", "/screenshot")
    (output / (name + ".png")).write_bytes(base64.b64decode(b))


def start_session(storage_ok=True):
    global session
    r = request(
        "POST",
        "/session",
        {
            "capabilities": {
                "alwaysMatch": {
                    "webkitgtk:browserOptions": {"binary": BINARY, "args": []}
                }
            }
        },
        timeout=60,
    )
    session = r["sessionId"]
    wd("POST", "/timeouts", {"script": 15000, "implicit": 0, "pageLoad": 30000})
    wait(
        lambda: selector("[data-testid=desktop-workspace-layout]"),
        "desktop native render",
        40,
    )
    assert invoke("gui_platform") == {"ok": True, "value": "linux"}
    if storage_ok:
        assert "無法讀取" not in text(), text()
    print("native WebKitGTK window + Linux IPC + desktop render PASS", flush=True)


def end_session():
    global session
    if session:
        wd("DELETE", "")
        session = None


def config():
    return home / "config" / args.application_id / "connections.toml"


def ensure_secret_encrypted(secret):
    content = config().read_text()
    assert secret not in content, "plaintext secret in TOML"
    assert config().stat().st_mode & 0o777 == 0o600


def verify_profiles():
    start_session()
    shot("workspace")
    click('[aria-label="軟體首頁"]')
    wait(lambda: "Linux" in text(), "Linux homepage label")
    shot("home")
    click("[data-testid=desktop-settings]")
    click("[data-testid=settings-connections]")
    wait(lambda: selector("[data-testid=runtime-host-form]"), "Host form")
    fill("[data-testid=name]", "Ubuntu native fixture")
    fill("[data-testid=base-url]", "https://runtime.example.test")
    fill("[data-testid=credential]", "ubuntu-host-secret-fixture")
    click("[data-testid=runtime-host-form] button[type=submit]")
    wait(lambda: selector("[data-host-id]"), "saved Host")
    assert js('return document.querySelector("[data-testid=credential]").value') == ""
    ensure_secret_encrypted("ubuntu-host-secret-fixture")
    print(
        "Host create via native UI, clear credential, private encrypted TOML PASS",
        flush=True,
    )
    click("[data-host-id] button")
    fill("[data-testid=name]", "Ubuntu native fixture edited")
    click("[data-testid=runtime-host-form] button[type=submit]")
    wait(lambda: "Ubuntu native fixture edited" in text(), "edited Host")
    click("[data-testid=settings-screens]")
    wait(lambda: selector("[data-testid=screen-form]"), "Screen form")
    fill("[data-testid=screen-name]", "Ubuntu Screen fixture")
    fill("[data-testid=screen-url]", "https://screen.example.test/mcp")
    fill("[data-testid=screen-token]", "ubuntu-screen-secret-fixture")
    click("[data-testid=screen-form] button[type=submit]")
    wait(lambda: selector("[data-testid^=saved-screen-]"), "saved Screen")
    ensure_secret_encrypted("ubuntu-screen-secret-fixture")
    assert js('return document.querySelector("[data-testid=screen-token]").value') == ""
    click("[data-testid^=saved-screen-] button:nth-child(2)")
    fill("[data-testid=screen-name]", "Ubuntu Screen fixture edited")
    click("[data-testid=screen-form] button[type=submit]")
    wait(lambda: "Ubuntu Screen fixture edited" in text(), "edited Screen")
    print("Screen create/edit via native UI + encrypted token PASS", flush=True)
    shot("screen-settings")
    end_session()
    start_session()
    click("[data-testid=desktop-settings]")
    click("[data-testid=settings-connections]")
    wait(lambda: "Ubuntu native fixture edited" in text(), "Host persists restart")
    click("[data-testid=settings-screens]")
    wait(lambda: "Ubuntu Screen fixture edited" in text(), "Screen persists restart")
    print("Actual app restart + Host/Screen reload PASS", flush=True)
    rows = js(
        'return Array.from(document.querySelectorAll("[data-testid^=saved-screen-] button")).map(b=>b.textContent)'
    )
    assert rows == ["測試", "編輯", "刪除"], rows
    click("[data-testid^=saved-screen-] button:last-child")
    wait(lambda: not selector("[data-testid^=saved-screen-]"), "delete Screen")
    click("[data-testid=settings-connections]")
    wait(lambda: selector("[data-host-id]"), "Host row")
    click("[data-host-id] button:last-of-type")
    click("[data-testid=confirm-delete]")
    wait(lambda: not selector("[data-host-id]"), "delete Host")
    print("Host + Screen delete via native UI PASS", flush=True)


def verify_clipboard():
    assert invoke(
        "plugin:clipboard-manager|write_text",
        {"text": "Ubuntu native clipboard fixture", "label": None},
    ) == {"ok": True, "value": None}
    reader = """import ctypes
G=ctypes.CDLL('libgtk-3.so.0');D=ctypes.CDLL('libgdk-3.so.0');L=ctypes.CDLL('libglib-2.0.so.0')
G.gtk_init_check.argtypes=[ctypes.c_void_p,ctypes.c_void_p];assert G.gtk_init_check(None,None)
D.gdk_atom_intern_static_string.argtypes=[ctypes.c_char_p];D.gdk_atom_intern_static_string.restype=ctypes.c_void_p
G.gtk_clipboard_get.argtypes=[ctypes.c_void_p];G.gtk_clipboard_get.restype=ctypes.c_void_p
G.gtk_clipboard_wait_for_text.argtypes=[ctypes.c_void_p];G.gtk_clipboard_wait_for_text.restype=ctypes.c_void_p
L.g_free.argtypes=[ctypes.c_void_p]
p=G.gtk_clipboard_wait_for_text(G.gtk_clipboard_get(D.gdk_atom_intern_static_string(b'CLIPBOARD')));assert p
print(ctypes.string_at(p).decode());L.g_free(p)
"""
    clipboard = subprocess.run(
        ["python3", "-c", reader],
        env=env,
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
    )
    assert clipboard.returncode == 0, clipboard.stderr
    assert clipboard.stdout.strip() == "Ubuntu native clipboard fixture", (
        clipboard.stdout
    )
    print(
        "Native clipboard IPC write + independent GTK clipboard read PASS", flush=True
    )


def verify_browser():
    applications = home / "data/applications"
    applications.mkdir(parents=True, exist_ok=True)
    handler = base / "browser.py"
    handler.write_text(
        "#!/usr/bin/python3\nimport json,sys\nfrom pathlib import Path\nPath("
        + repr(str(base / "browser-argv.json"))
        + ").write_text(json.dumps(sys.argv[1:]))\n"
    )
    handler.chmod(0o700)
    desktop = applications / "art-term-test-browser.desktop"
    desktop.write_text(
        "[Desktop Entry]\nType=Application\nName=Private test browser\nExec="
        + str(handler)
        + " %u\nMimeType=x-scheme-handler/http;x-scheme-handler/https;\nNoDisplay=true\n"
    )
    mime = home / "config/mimeapps.list"
    mime.write_text(
        "[Default Applications]\nx-scheme-handler/http=art-term-test-browser.desktop\nx-scheme-handler/https=art-term-test-browser.desktop\n"
    )
    url = "https://example.test/a?q=%22;$(touch%20x)&literal=hello#fragment"
    assert invoke("gui_open_web_url", {"url": url}) == {"ok": True, "value": None}
    wait(lambda: (base / "browser-argv.json").exists(), "native GIO browser")
    assert json.loads((base / "browser-argv.json").read_text()) == [url]
    (base / "browser-argv.json").unlink()
    assert invoke("gui_open_web_url", {"url": "file:///tmp/no-launch"})["ok"] is False
    time.sleep(0.2)
    assert not (base / "browser-argv.json").exists()
    print(
        "Native GIO HTTP(S) handler exact single argv + unsafe-scheme refusal PASS",
        flush=True,
    )
    end_session()


def verify_keyring_failure():
    before = config().read_bytes()
    subprocess.run(
        [
            "gdbus",
            "call",
            "--session",
            "--dest",
            "org.freedesktop.secrets",
            "--object-path",
            "/org/freedesktop/secrets",
            "--method",
            "org.freedesktop.Secret.Service.SetAlias",
            "default",
            "/org/freedesktop/secrets/collection/session",
        ],
        env=env,
        check=True,
        capture_output=True,
    )
    alias = subprocess.run(
        [
            "gdbus",
            "call",
            "--session",
            "--dest",
            "org.freedesktop.secrets",
            "--object-path",
            "/org/freedesktop/secrets",
            "--method",
            "org.freedesktop.Secret.Service.ReadAlias",
            "default",
        ],
        env=env,
        check=True,
        capture_output=True,
    )
    assert b"collection/session" in alias.stdout
    start_session(storage_ok=False)
    click("[data-testid=desktop-settings]")
    click("[data-testid=settings-connections]")
    wait(lambda: "[STORAGE]" in text(), "sanitized storage failure UI")
    failure = invoke("gui_runtime_hosts")
    assert failure["ok"] is False and failure["error"]["kind"] == "storage", failure
    fill("[data-testid=name]", "Blocked Host")
    fill("[data-testid=base-url]", "https://blocked.example.test")
    fill("[data-testid=credential]", "must-not-save-fixture")
    click("[data-testid=runtime-host-form] button[type=submit]")
    wait(
        lambda: (
            "[STORAGE]" in text()
            and js(
                "return !document.querySelector('[data-testid=runtime-host-form] button[type=submit]').disabled && document.querySelector('[data-testid=credential]').value === '';"
            )
        ),
        "save settled with storage refusal",
    )
    rejected = invoke(
        "gui_runtime_save_host",
        {
            "input": {
                "name": "Blocked Host",
                "baseUrl": "https://blocked.example.test",
                "credential": "must-not-save-fixture",
                "category": "host",
            }
        },
    )
    assert rejected["ok"] is False and rejected["error"]["kind"] == "storage", rejected
    assert config().read_bytes() == before
    shot("storage-failure")
    assert str(home) not in text() and "must-not-save-fixture" not in text()
    print(
        "Nonpersistent keyring default sanitized storage error + writes refused + GUI available PASS",
        flush=True,
    )
    end_session()


def main():
    spawn(
        [
            "dbus-daemon",
            "--session",
            "--nofork",
            "--nopidfile",
            "--address=" + env["DBUS_SESSION_BUS_ADDRESS"],
        ],
        "bus",
    )
    wait(lambda: (base / "bus").exists(), "private bus")
    spawn(
        ["gnome-keyring-daemon", "--foreground", "--components=secrets", "--unlock"],
        "keyring",
        b"fixture-only-password",
    )

    def keyring_ready():
        r = subprocess.run(
            [
                "gdbus",
                "call",
                "--session",
                "--dest",
                "org.freedesktop.secrets",
                "--object-path",
                "/org/freedesktop/secrets",
                "--method",
                "org.freedesktop.Secret.Service.ReadAlias",
                "default",
            ],
            env=env,
            check=False,
            capture_output=True,
        )
        return r.returncode == 0 and b"collection/login" in r.stdout

    wait(keyring_ready, "persistent keyring")
    authority = base / "Xauthority"
    cookie = secrets.token_hex(16)
    subprocess.run(
        ["xauth", "-f", str(authority), "add", ":0", ".", cookie],
        env=env,
        check=True,
        capture_output=True,
    )
    display_read, display_write = os.pipe()
    try:
        spawn(
            [
                "Xvfb",
                "-displayfd",
                str(display_write),
                "-screen",
                "0",
                "1440x1000x24",
                "-nolisten",
                "tcp",
                "-auth",
                str(authority),
            ],
            "xvfb",
            pass_fds=(display_write,),
        )
        os.close(display_write)
        display_write = None
        if not select.select([display_read], [], [], 10)[0]:
            raise RuntimeError("Xvfb did not allocate a display")
        display = os.read(display_read, 64).decode().strip()
        if not display.isdecimal():
            raise RuntimeError("Invalid Xvfb display number")
    finally:
        os.close(display_read)
        if display_write is not None:
            os.close(display_write)
    env["DISPLAY"] = ":" + display
    env["XAUTHORITY"] = str(authority)
    subprocess.run(
        ["xauth", "-f", str(authority), "add", env["DISPLAY"], ".", cookie],
        env=env,
        check=True,
        capture_output=True,
    )
    wait(
        lambda: (
            subprocess.run(
                ["xdotool", "getdisplaygeometry"],
                env=env,
                capture_output=True,
                check=False,
            ).returncode
            == 0
        ),
        "private X display",
    )
    driver = spawn(["WebKitWebDriver", "--port=" + str(port)], "webdriver")
    time.sleep(0.2)
    if driver.poll() is not None:
        raise RuntimeError("WebKitWebDriver exited before opening its private port")
    wait(
        lambda: driver.poll() is None and request("GET", "/status")["ready"],
        "native driver",
    )
    if driver.poll() is not None:
        raise RuntimeError("WebKitWebDriver exited")
    verify_profiles()
    verify_clipboard()
    verify_browser()
    verify_keyring_failure()
    (output / "result.json").write_text(
        json.dumps(
            {
                "result": "PASS",
                "binary": BINARY,
                "binarySha256": hashlib.sha256(
                    pathlib.Path(BINARY).read_bytes()
                ).hexdigest(),
                "sandboxDisabled": env.get("WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS")
                == "1",
                "checks": [
                    "native WebKitGTK render",
                    "Linux platform/homepage",
                    "Host CRUD native UI",
                    "Screen CRUD native UI",
                    "app restart persistence",
                    "encrypted private TOML",
                    "native GIO exact argv and scheme guard",
                    "native clipboard GTK readback",
                    "Secret Service failure UI and refused write",
                ],
            },
            indent=2,
        )
        + "\n"
    )


try:
    main()
finally:
    try:
        end_session()
    except (OSError, RuntimeError) as error:
        print("Session cleanup: " + str(error), file=sys.stderr)
    for p in reversed(processes):
        if p.poll() is None:
            try:
                os.killpg(p.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
    for p in reversed(processes):
        try:
            p.wait(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(p.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            p.wait()
    shutil.rmtree(base)
