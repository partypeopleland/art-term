#!/usr/bin/env python3
"""Exercise the real Crossterm EventStream through a private Unix PTY."""
import argparse
import array
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
import termios
import tempfile
import time


def exercise(binary, chunks, delay=0.003, resize=False, terminate=False):
    master, slave = pty.openpty()
    original = termios.tcgetattr(slave)
    original_flags = fcntl.fcntl(slave, fcntl.F_GETFL)
    temporary = tempfile.TemporaryDirectory(prefix="art-term-terminal-events-", dir="/tmp")
    audit = Path(temporary.name) / "fd.json"
    fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 36, 120, 1200, 720))
    raw = bytearray()
    process = None
    def child():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)
    def pump(timeout=0.02):
        if select.select([master], [], [], timeout)[0]:
            try: raw.extend(os.read(master, 65536))
            except OSError: pass
    try:
        env = dict(os.environ, ART_TERM_EVENTS_FD_AUDIT=str(audit))
        process = subprocess.Popen([str(binary)], stdin=slave, stdout=slave, stderr=slave,
                                   preexec_fn=child, close_fds=True, env=env)
        deadline = time.monotonic() + 5
        while b"READY\n" not in raw:
            pump()
            assert process.poll() is None, bytes(raw)
            assert time.monotonic() < deadline, "EventStream did not start"
        descriptors = json.loads(audit.read_text())
        assert descriptors["stdin_flags"] == original_flags
        assert descriptors["owned_tty_flags"] and all(flags & os.O_NONBLOCK for flags in descriptors["owned_tty_flags"]), descriptors
        for chunk in chunks:
            os.write(master, chunk)
            until = time.monotonic() + .2
            while time.monotonic() < until:
                pending = array.array("i", [0])
                fcntl.ioctl(slave, termios.FIONREAD, pending, True)
                if pending[0] == 0: break
                pump(.001)
            if delay: time.sleep(delay)
        if resize:
            fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 42, 132, 1320, 840))
            os.kill(process.pid, signal.SIGWINCH)
            time.sleep(.03)
        if terminate:
            os.kill(process.pid, signal.SIGTERM)
        else:
            os.write(master, b"\x11")
        deadline = time.monotonic() + 5
        while process.poll() is None:
            pump()
            assert time.monotonic() < deadline, "EventStream fixture did not exit"
        pump(0)
        assert process.returncode == 0, bytes(raw)
        assert termios.tcgetattr(slave) == original, "TTY settings were not restored"
        assert fcntl.fcntl(slave, fcntl.F_GETFL) == original_flags, "Shared stdin flags changed"
        events = [json.loads(line) for line in bytes(raw).splitlines() if line.startswith(b"{")]
        if not terminate:
            assert events[-1] == {"kind":"key", "code":"Char('q')", "mods":2}, events
            events.pop()
        return events
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            try: process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill(); process.wait()
        os.close(master); os.close(slave)
        temporary.cleanup()


def closed_pty(binary):
    master, slave = pty.openpty()
    def child():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)
        signal.signal(signal.SIGHUP, signal.SIG_IGN)
    process = subprocess.Popen([str(binary)], stdin=slave, stdout=slave, stderr=slave,
                               preexec_fn=child, close_fds=True)
    try:
        raw = bytearray()
        deadline = time.monotonic() + 5
        while b"READY\n" not in raw:
            assert process.poll() is None
            assert time.monotonic() < deadline
            if select.select([master], [], [], .05)[0]: raw.extend(os.read(master, 65536))
        os.close(master)
        master = None
        assert process.wait(timeout=2) != 0, "Closed input was not surfaced as an I/O error"
    finally:
        if process.poll() is None:
            process.terminate()
            try: process.wait(timeout=2)
            except subprocess.TimeoutExpired: process.kill(); process.wait()
        if master is not None: os.close(master)
        os.close(slave)


def key(char, modifiers=0):
    return {"kind":"key", "code":f"Char('{char}')", "mods":modifiers}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=False)
    checks = []
    def passed(name):
        checks.append(name)
        print("PASS " + name, flush=True)
    reports = [
        (b"\x1b[?61;4;6c", {"kind":"report", "type":"da", "values":[61,4,6]}),
        (b"\x1b[6;20;10t", {"kind":"report", "type":"cell", "width":10, "height":20}),
        (b"\x1b[4;720;1200t", {"kind":"report", "type":"pixels", "width":1200, "height":720}),
        (b"\x1b[8;36;120t", {"kind":"report", "type":"cells", "columns":120, "rows":36}),
    ]
    boundaries = 0
    for payload, expected in reports:
        for split in range(1, len(payload)):
            actual = exercise(binary, [payload[:split], payload[split:], b"Z"])
            assert actual == [expected, key("Z", 1)], (split, payload, actual)
            boundaries += 1
    passed(f"All {boundaries} report split boundaries preserve replies and following keys")
    actual = exercise(binary, [b"\x1b", b"[?61;4c", b"Z"], delay=.03)
    assert actual == [{"kind":"report", "type":"da", "values":[61,4]}, key("Z",1)], actual
    passed("A separate ESC read with 30ms delay remains a terminal report")
    actual = exercise(binary, [b"\x1b[?61;", b"4c", b"Z"], delay=1.1)
    assert actual == [{"kind":"report", "type":"da", "values":[61,4]}, key("Z",1)], actual
    passed("Partial CSI survives beyond the image-probe deadline without key leakage")
    for bad in [b"\x1b[?1x", b"\x1b[?61;65536c", b"\x1b[6;1;65536t", b"\x1b[6;1;2;3t",
                b"\x1b[?" + b"4;" * 32 + b"4c", b"\x1b[?" + b"1;" * 2048 + b"c",
                b"\x1b[6;" + b"1;" * 2048 + b"t"]:
        actual = exercise(binary, [bad, b"Z"])
        assert actual == [key("Z",1)], (bad[:20], actual)
    passed("Malformed and oversized report metadata is consumed before the following key")
    paste = "中文\x1b[?61;4c\x1b[6;20;10t"
    actual = exercise(binary, [b"\x1b[200~" + paste.encode() + b"\x1b[201~", "中".encode(),
        b"\x1d", b"\x1b[A", b"\x1bz", b"\x1b[<0;20;10M", b"\x1b[<32;21;10M",
        b"\x1b[<0;21;10m", b"\x1b[<64;21;10M"])
    assert actual == [{"kind":"paste", "text":paste}, key("中"), key("5",2),
        {"kind":"key", "code":"Up", "mods":0}, key("z",4),
        {"kind":"mouse", "event":"Down(Left)", "column":19,"row":9},
        {"kind":"mouse", "event":"Drag(Left)", "column":20,"row":9},
        {"kind":"mouse", "event":"Up(Left)", "column":20,"row":9},
        {"kind":"mouse", "event":"ScrollUp", "column":20,"row":9}], actual
    passed("Paste reply-shaped text, CJK, control/Alt/arrows and mouse events retain exact order")
    started = time.monotonic()
    actual = exercise(binary, [b"\x1b"], delay=.09)
    assert actual == [{"kind":"key", "code":"Esc", "mods":0}], actual
    assert time.monotonic() - started < 1
    passed("Standalone Escape flushes and does not block input")
    actual = exercise(binary, [b"z"], resize=True)
    assert key("z") in actual and {"kind":"resize", "columns":132,"rows":42} in actual, actual
    passed("SIGWINCH keeps normal resize events")
    assert exercise(binary, [], terminate=True) == []
    passed("Cancellation wakes the same reader and restores the complete TTY snapshot")
    for _ in range(20):
        closed_pty(binary)
    passed("Closed PTY input exits with an error in 20 runs without spinning or holding the reader")
    passed("Every run uses an owned nonblocking input descriptor and preserves stdin flags")
    (output / "result.json").write_text(json.dumps({"result":"PASS", "binary_sha256":hashlib.sha256(binary.read_bytes()).hexdigest(),
        "checks":checks, "report_boundaries":boundaries, "eof_repetitions":20, "escape_ambiguity_ms":50},indent=2)+"\n")


if __name__ == "__main__":
    main()
