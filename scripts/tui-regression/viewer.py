#!/usr/bin/env python3
"""Actual PTY viewer regression against isolated loopback Runtime/MCP fixtures.

Committed UTF-8 and physical cursor positioning are covered; physical OS IME
composition and rendering by an actual Sixel-capable client are not exercised.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import select
import struct
import tempfile
import time
import tomllib
import traceback
import zlib

spec = importlib.util.spec_from_file_location("tui_driver", Path(__file__).with_name("run.py"))
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)


class ObservedTerminal(driver.Terminal):
    watch_instructions = False
    instruction_frames = 0

    def pump(self, timeout=0.05):
        super().pump(timeout)
        if self.watch_instructions and "點擊右上 [Desktop]" in self.text():
            self.instruction_frames += 1


def decode_sixel(payload):
    from native_images import decode_sixel as decode
    raster = decode(payload)
    colors = {raster.pixels[i:i + 3] for i in range(0, len(raster.pixels), 3)}
    assert len(colors) >= 4, "Encoded image lost the known source colors"
    for expected in ((240, 30, 20), (30, 40, 240)):
        assert any(all(abs(a - b) <= 4 for a, b in zip(color, expected)) for color in colors), ("Missing source corner", expected)
    return raster.summary()


def native_probe(binary, output, mode="sixel"):
    result = {"binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "mode": "sixel-protocol-only", "configured_mode": mode,
              "checks": [], "physical_os_ime": "NOT_RUN", "actual_sixel_client": "NOT_RUN"}
    app = runtime = mcp = None
    with tempfile.TemporaryDirectory(prefix="art-term-viewer-native-") as directory:
        root = Path(directory)
        home, fixtures = root / "home", root / "fixtures"
        home.mkdir(mode=0o700)
        fixtures.mkdir(mode=0o700)
        settings = home / "config/art-term-tui/display.toml"
        settings.parent.mkdir(parents=True, mode=0o700)
        config = f"# isolated protocol fixture: explicit cell pixels\nimage_mode='{mode}'\ncell_width=8\ncell_height=16\n"
        settings.write_text(config)
        settings.chmod(0o600)
        try:
            runtime = driver.Runtime(fixtures)
            mcp = driver.McpFixture("A")
            mcp.image = detailed_png()
            app = driver.Terminal(binary, home, runtime.ca)
            app.start()
            assert settings.read_text() == config, "existing display config overwritten"
            app.click_text("New Host")
            app.wait_text("HTTPS Runtime URL")
            app.send("Native fixture\t" + runtime.origin + "\t" + runtime.credential)
            app.send(b"\x13")
            app.wait(lambda: "HTTPS Runtime URL" not in app.text(), "Host saved")
            app.idle()
            app.action("New Screen")
            app.wait_text("新增 Screen")
            app.send("Native\t" + mcp.url + "\t" + mcp.token)
            app.send(b"\x13")
            app.wait(lambda: "新增 Screen" not in app.text(), "Screen saved")
            app.idle()
            app.click_text("New Shell", (0, 1, 40, 24))
            app.wait(lambda: len(runtime.work()) == 1, "work created")
            app.wait_text("Live")
            app.bind_screen(1)
            app.wait(lambda: bool(app.sixels), "actual thumbnail Sixel", timeout=40)
            first_frame_at = time.monotonic()
            result["thumbnail_raster"] = decode_sixel(app.sixels[-1])
            mcp.capture_started.clear()
            mcp.capture_gate.clear()
            app.wait(mcp.capture_started.is_set, "native gated capture", timeout=40)
            result["next_capture_wait_seconds"] = time.monotonic() - first_frame_at
            assert result["next_capture_wait_seconds"] >= 29, "preview polled before its thirty-second interval"

            def open_viewer():
                before = len(app.sixels)
                app.click(15, 27)
                app.wait_text("[X]")
                def raster_width(payload):
                    return int(re.search(rb'"\d+;\d+;(\d+);', payload)[1])
                app.wait(lambda: any(raster_width(payload) > result["thumbnail_raster"]["width"] for payload in app.sixels[before:]), "actual expanded Sixel")
                largest = max(app.sixels[before:], key=raster_width)
                result["expanded_raster"] = decode_sixel(largest)

            def cleared(action, label):
                start = len(app.raw)
                action()
                app.wait(lambda: b"\x1b[2J" in app.raw[start:], label + " erases native raster")

            open_viewer()
            assert result["expanded_raster"]["width"] > result["thumbnail_raster"]["width"]
            cleared(lambda: app.click_text("[X]", (40, 1, 120, 4)), "X")
            open_viewer()
            cleared(lambda: app.prefix("c"), "modal")
            app.wait_text("Stop")
            app.send(b"\x1b")
            app.wait(lambda: "Stop" not in app.text(), "modal dismissed")
            open_viewer()
            cleared(lambda: app.resize(32, 110), "resize")
            app.resize(36, 120)
            app.settle(.3)
            cleared(lambda: app.prefix("q"), "exit")
            app.finish()
            assert app.probe_queries == ["c", "16t", "14t", "18t"], app.probe_queries
            assert b"\x1b[6n" not in app.raw and b"\x1b_G" not in app.raw, "unexpected cursor/Kitty query"
            assert b"allow-passthrough" not in app.raw
            assert settings.read_text() == config
            result["checks"].append({"id": "T-IMAGE-02", "result": "PASS", "description": "Actual PTY Sixel RGB raster decoded within bounds; original colors; explicit config preserved; X/modal/resize/exit erase; one bounded native capability/pixel probe and no cursor query"})
            app.save(output, "native-protocol")
            print("PASS T-IMAGE-02 actual Sixel protocol and cleanup (client rendering NOT_RUN)", flush=True)
        except Exception:
            result["failure"] = traceback.format_exc()
            print(result["failure"], flush=True)
            if app:
                app.save(output, "native-failure")
        finally:
            if app:
                result["standard_cursor_dsr_queries"] = app.raw.count(b"\x1b[6n")
            if mcp:
                mcp.capture_gate.set()
            if app:
                app.close()
            if mcp:
                mcp.close()
            if runtime:
                runtime.close()
    (output / "result.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    return int("failure" in result)


def detailed_png():
    """Original 1280x640 source with colored corners and a one-pixel grating."""
    def chunk(kind, data):
        return struct.pack("!I", len(data)) + kind + data + struct.pack("!I", zlib.crc32(kind + data) & 0xffffffff)
    rows = []
    for y in range(640):
        row = bytearray(b"\0")
        for x in range(1280):
            color = ((240, 30, 20), (20, 220, 30), (30, 40, 240), (240, 210, 20))[(y >= 320) * 2 + (x >= 640)]
            if 384 <= x < 896 and 192 <= y < 448:
                color = (250, 0, 250) if x % 2 else (0, 250, 250)
            row.extend(color)
        rows.append(row)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack("!IIBBBBB", 1280, 640, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(b"".join(rows))) + chunk(b"IEND", b"")


def policy_probe(binary, output):
    result = {"binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "checks": [], "mode": "terminal-detection"}
    app = runtime = mcp = None
    with tempfile.TemporaryDirectory(prefix="art-term-preview-detection-") as directory:
        root = Path(directory)
        fixture = root / "fixture"; fixture.mkdir(mode=0o700)
        no_pixels = {"16t": None, "14t": None, "18t": None}
        cases = [
            ("SSH-supported", {"SSH_CONNECTION": "fixture"}, "auto", {}, True),
            ("SSH-client-supported", {"SSH_CLIENT": "fixture"}, "auto", {}, True),
            ("SSH-legacy-supported", {"SSH_TTY": "/dev/pts/fixture"}, "compatible", {}, True),
            ("generic-terminal-supported", {"TERM": "xterm-256color", "XTERM_VERSION": None}, "auto", {}, True),
            ("negative-da", {"SSH_CONNECTION": "fixture"}, "sixel", {"c": b"\x1b[?61;6c"}, False),
            ("class-is-not-capability", {}, "auto", {"c": b"\x1b[?4c"}, False),
            ("timeout-no-da", {}, "auto", {"c": None}, False),
            ("timeout-no-replies", {}, "auto", {name: None for name in ("c", "16t", "14t", "18t")}, False),
            ("late-partial-da", {"SSH_TTY": "fixture"}, "auto", {"c": [(.2, b"\x1b[?61;"), (1.5, b"4c")]}, False),
            ("missing-pixels", {}, "auto", no_pixels, False),
            ("zero-pixels", {}, "auto", {**no_pixels, "16t": b"\x1b[6;0;0t"}, False),
            ("area-fallback", {}, "auto", {"16t": None}, True),
            ("inexact-area", {}, "auto", {"16t": None, "14t": b"\x1b[4;577;960t"}, False),
            ("invalid-settings", {}, "unknown", {}, False),
        ]
        try:
            runtime = driver.Runtime(fixture)
            mcp = driver.McpFixture("A")
            for index, (name, env, mode, reports, supported) in enumerate(cases):
                home = root / ("home-" + name); home.mkdir(mode=0o700)
                settings = home / "config/art-term-tui/display.toml"
                settings.parent.mkdir(parents=True, mode=0o700)
                content = f"image_mode='{mode}'\n"
                settings.write_text(content); settings.chmod(0o600)
                app = driver.Terminal(binary, home, runtime.ca, env_overrides=env,
                                      terminal_reports=reports, tty_pixels=(640, 480))
                app.start()
                if mode != "unknown":
                    app.wait(lambda: len(app.probe_queries) == 4, "actual probe queries " + name, timeout=4)
                    assert app.probe_queries == ["c", "16t", "14t", "18t"], app.probe_queries
                if supported:
                    app.settle(.15)
                    app.wait(lambda: "正在偵測" not in app.text() and "未啟用" not in app.text(), "positive terminal detection " + name, timeout=4)
                    assert "正在偵測" not in app.text() and "未啟用" not in app.text()
                else:
                    app.wait(lambda: "未啟用" in app.text() and "桌面工具仍可使用" in "".join(app.text().replace("│", "").split()), "unavailable preview notice " + name, timeout=4)
                    if name == "late-partial-da":
                        app.settle(1)
                        assert "未啟用" in app.text(), "late DA changed closed probe result"
                    assert not app.sixels
                assert settings.read_text() == content, "existing display file changed"
                if index in (0, 4):
                    app.click_text("New Host"); app.wait_text("HTTPS Runtime URL")
                    app.send("SSH detection fixture\t" + runtime.origin + "\t" + runtime.credential); app.send(b"\x13")
                    app.wait(lambda: "HTTPS Runtime URL" not in app.text(), "Host saved"); app.idle()
                    app.action("New Screen"); app.wait_text("新增 Screen")
                    app.send("SSH desktop\t" + mcp.url + "\t" + mcp.token); app.send(b"\x13")
                    app.wait(lambda: "新增 Screen" not in app.text(), "Screen saved"); app.idle()
                    app.click_text("New Shell", (0, 1, 40, 24)); app.wait_text("Live")
                    captures = len([event for event in mcp.events if event.get("method") == "tools/call"])
                    app.bind_screen(1); app.idle()
                    if supported:
                        app.wait(lambda: bool(app.sixels), "supported SSH native image", timeout=40)
                        raster = __import__("native_images").decode_sixel(app.sixels[-1])
                        assert raster.width == 38 * 8 and raster.height == (9 * 16 // 6 * 6), raster.summary()
                    else:
                        app.settle(.4)
                        assert len([event for event in mcp.events if event.get("method") == "tools/call"]) == captures, "unavailable preview requested screenshot"
                        assert not app.sixels, "unavailable preview emitted image"
                    app.prefix("t"); app.send("SSH-SHELL-STILL-USABLE")
                    app.wait(lambda: any("SSH-SHELL-STILL-USABLE" in state["input"] for state in runtime.work().values()), "ordinary SSH work input")
                app.prefix("q"); app.finish(); app.save(output, "detect-" + name)
                if mode == "unknown": assert not app.probe_queries
                assert b"\x1b[6n" not in app.raw and b"\x1b_G" not in app.raw
                app.close(); app = None
                result["checks"].append({"id": "T-IMAGE-DETECT", "result": "PASS", "description": name + (" enables native preview" if supported else " refuses only preview") + ", preserves settings and TUI"})
                print("PASS T-IMAGE-DETECT " + name, flush=True)
        except Exception:
            result["failure"] = traceback.format_exc(); print(result["failure"], flush=True)
            if app: app.save(output, "detection-failure")
        finally:
            if app: app.close()
            if mcp: mcp.close()
            if runtime: runtime.close()
    (output / "result.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    return int("failure" in result)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--mode", choices=("native", "sixel", "legacy", "policy"), default="native")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=False)
    if args.mode in ("sixel", "legacy"):
        return native_probe(binary, output, "compatible" if args.mode == "legacy" else "sixel")
    if args.mode == "policy":
        return policy_probe(binary, output)
    result = {"binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "checks": [],
              "mode": "native-sixel", "physical_os_ime": "NOT_RUN", "actual_sixel_client": "NOT_RUN"}
    app = runtime = None
    mcps = []

    def passed(case, description):
        result["checks"].append({"id": case, "result": "PASS", "description": description})
        print("PASS " + case + " " + description, flush=True)

    def cursor():
        return (app.screen.cursor.x, app.screen.cursor.y, app.screen.cursor.hidden)

    def signature(region):
        left, top, right, bottom = region
        return (tuple((app.screen.buffer[y][x].data, app.screen.buffer[y][x].fg, app.screen.buffer[y][x].bg)
                      for y in range(top, bottom) for x in range(left, right)),
                app.image_signature(region))

    def expanded():
        return bool(app.locations("[X]", (40, 1, app.screen.columns, 4)))

    def toggle():
        app.click(15, 27)
        app.settle(.25)

    def drag(start, end):
        app.send(f"\x1b[<0;{start[0]+1};{start[1]+1}M\x1b[<32;{end[0]+1};{end[1]+1}M\x1b[<0;{end[0]+1};{end[1]+1}m")
        app.settle(.08)

    with tempfile.TemporaryDirectory(prefix="art-term-viewer-test-") as directory:
        root = Path(directory)
        home, fixtures = root / "home", root / "fixtures"
        home.mkdir(mode=0o700)
        fixtures.mkdir(mode=0o700)
        try:
            runtime = driver.Runtime(fixtures)
            mcps = [driver.McpFixture("A"), driver.McpFixture("B", sse=True)]
            mcps[0].image = detailed_png()
            mcps[1].image = driver.screenshot_png((20, 30, 240))
            app = ObservedTerminal(binary, home, runtime.ca)
            app.start()
            settings = home / "config/art-term-tui/display.toml"
            assert tomllib.loads(settings.read_text())["image_mode"] == "auto"
            assert settings.stat().st_mode & 0o777 == 0o600
            passed("T-IMAGE-03", "Fresh startup creates private auto-native display configuration")
            app.click_text("New Host")
            app.wait_text("HTTPS Runtime URL")
            app.send("中文")
            app.wait_text("中文")
            app.settle(.15)
            anchor = app.locations("中文")[0]
            assert cursor() == (anchor[0] + 2, anchor[1], False), ("form CJK insertion cursor", cursor(), anchor)
            passed("T-INPUT-03", "Local form exposes exact visible CJK insertion anchor")
            app.send("Viewer fixture\t" + runtime.origin + "\t" + runtime.credential)
            app.send(b"\x13")
            app.wait(lambda: "HTTPS Runtime URL" not in app.text(), "Host saved")
            app.idle()
            for label, mcp in zip(("Detailed", "Blue"), mcps):
                app.action("New Screen")
                app.wait_text("新增 Screen")
                app.send(label + "\t" + mcp.url + "\t" + mcp.token)
                app.send(b"\x13")
                app.wait(lambda: "新增 Screen" not in app.text(), "Screen saved")
                app.idle()
            app.click_text("New Shell", (0, 1, 40, 24))
            app.wait(lambda: len(runtime.work()) == 1, "first work")
            first = next(iter(runtime.work()))
            app.wait(lambda: runtime.work()[first]["bootstrap"], "first bootstrap")
            app.wait_text("Live")
            app.prefix("t")
            runtime.emit(first, b"\x1b[2J\x1b[3;5H")
            app.settle(.2)
            assert cursor() == (45, 5, False), ("remote insertion cursor", cursor())
            borders = signature((0, 1, 1, 34)) + signature((39, 1, 40, 34))
            app.send("中文輸入測試")
            app.wait(lambda: "中文輸入測試" in runtime.work()[first]["input"], "committed CJK received")
            app.wait_text("中文輸入測試")
            app.settle(.15)
            assert cursor() == (57, 5, False), ("CJK-width insertion cursor", cursor())
            assert borders == signature((0, 1, 1, 34)) + signature((39, 1, 40, 34)), "left borders moved"
            app.save(output, "cjk-cursor")
            passed("T-INPUT-01", "Committed CJK reaches work; visible physical cursor uses exact display-cell anchor; left borders stable")

            app.bind_screen(1)
            app.wait(lambda: bool(app.image_cells("f01e14")), "source thumbnail", timeout=40)
            mcps[0].capture_started.clear()
            mcps[0].capture_gate.clear()
            app.wait(mcps[0].capture_started.is_set, "gated next capture", timeout=40)
            thumbnail = signature((1, 24, 39, 33))
            before_frames = len(runtime.frames)
            before_resizes = list(runtime.work()[first]["resizes"])
            before_calls = len(mcps[0].events)
            app.watch_instructions = True
            toggle()
            app.wait(expanded, "right viewer opens")
            app.wait(lambda: bool([p for p in app.image_cells("f01e14") if p[0] > 40]), "right image prepared", timeout=40)
            assert thumbnail == signature((1, 24, 39, 33)), "thumbnail changed during gated click"
            assert app.instruction_frames == 0, "desktop instructions flashed in observed PTY frame"
            assert "Connections" in app.text() and "1:" in app.text(), "left pane or tabs lost"
            passed("T-VIEWER-01", "Cached thumbnail survives gated capture; click expands only right pane and preserves tabs")

            region = (42, 4, 118, 32)
            fit = signature(region)
            assert not app.image_cells("fa00fa") and not app.image_cells("00fafa"), "test source detail was already resolved at Fit"
            app.send(b"LOCAL-VIEWER\x1b[200~PASTED-VIEWER\x1b[201~")
            app.settle(.1)
            app.wheel(80, 18, -16)
            app.wait(lambda: signature(region) != fit, "zoom changes original source crop")
            zoomed = signature(region)
            assert len(app.image_cells("fa00fa")) + len(app.image_cells("00fafa")) > 10, "high-frequency source detail did not resolve at zoom"
            drag((80, 18), (102, 23))
            app.wait(lambda: signature(region) != zoomed, "drag changes crop")
            for _ in range(32):
                drag((80, 18), (117, 31))
            app.settle(.2)
            clamped = signature(region)
            drag((80, 18), (117, 31))
            app.settle(.2)
            assert signature(region) == clamped, "image did not clamp at edge"
            app.click_text("[Fit]", (40, 1, 120, 4))
            app.wait(lambda: signature(region) == fit, "Fit restores full original source")
            assert len(runtime.frames) == before_frames, "viewer gestures sent work PTY bytes"
            assert runtime.work()[first]["resizes"] == before_resizes, "viewer gestures resized remote PTY"
            assert len(mcps[0].events) == before_calls, "presentation caused MCP capture"
            assert app.instruction_frames == 0, "desktop instructions flashed during gestures"
            app.save(output, "expanded-fit")
            passed("T-IMAGE-01", "Original high-frequency PNG resolves when zoomed; drag, clamp and Fit affect local crop only")

            runtime.emit(first, b"\r\nBACKGROUND-VIEWER-OUTPUT\r\n")
            app.settle(.15)
            app.click_text("[X]", (40, 1, 120, 4))
            app.wait(lambda: not expanded(), "X closes")
            app.wait_text("BACKGROUND-VIEWER-OUTPUT")
            toggle()
            app.wait(expanded, "reopen")
            toggle()
            app.wait(lambda: not expanded(), "thumbnail closes")
            passed("T-VIEWER-02", "X and repeated thumbnail click close; background terminal output is retained")
            app.watch_instructions = False
            mcps[0].capture_gate.set()

            toggle()
            app.wait(expanded, "open before modal")
            app.prefix("c")
            app.wait_text("Stop")
            assert not expanded(), "screen modal retained viewer"
            app.send(b"\x1b")
            app.wait(lambda: "Stop" not in app.text(), "modal dismissed")
            app.wait(lambda: bool(app.image_cells("f01e14")), "thumbnail after modal", timeout=40)
            toggle()
            app.wait(expanded, "open before tab switch")
            app.prefix("n")
            app.wait(lambda: len(runtime.work()) == 2, "second work")
            app.wait_text("2:")
            app.idle()
            app.wait(lambda: not app.image_cells("f01e14"), "new tab clears old source")
            assert not expanded() and not app.image_cells("f01e14"), "new tab retained old source"
            app.bind_screen(2)
            app.wait(lambda: bool(app.image_cells("141ef0")), "second source", timeout=40)
            toggle()
            app.wait(expanded, "blue viewer")
            app.prefix("c")
            app.wait_text("Stop")
            app.send(b"\x1b[B\r")
            app.wait(lambda: bool(app.image_cells("f01e14")), "source switch to detailed", timeout=40)
            assert not expanded() and not app.image_cells("141ef0"), "source switch retained old image"
            toggle()
            app.wait(expanded, "open before resize")
            app.resize(32, 110)
            app.settle(.4)
            assert "Connections" in app.text() and "2:" in app.text(), "resize corrupted panes"
            app.resize(36, 120)
            app.settle(.4)
            app.bind_screen(0)
            app.wait(lambda: not expanded() and not app.image_cells("f01e14"), "Stop clears viewer")
            passed("T-VIEWER-03", "Modal, tab, source, resize and Stop lifecycle preserve pane boundaries and discard stale source")
            app.prefix("q")
            app.finish()
            passed("T-INPUT-02", "Clean exit restores original terminal modes")
        except Exception:
            result["failure"] = traceback.format_exc()
            print(result["failure"], flush=True)
            if app:
                app.save(output, "failure")
        finally:
            for mcp in mcps:
                mcp.capture_gate.set()
            if app:
                app.close()
            for mcp in mcps:
                mcp.close()
            if runtime:
                runtime.close()
    (output / "result.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    return int("failure" in result)


if __name__ == "__main__":
    raise SystemExit(main())
