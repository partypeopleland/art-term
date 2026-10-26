#!/usr/bin/env python3
"""Verify right-terminal wheel routing through a real TUI and isolated Runtime fixture."""
import argparse
import hashlib
import json
from pathlib import Path
import tempfile
import traceback

from run import Runtime, Terminal


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output-dir', type=Path, required=True)
    args = parser.parse_args()
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=False)
    result = {'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(), 'checks': []}
    with tempfile.TemporaryDirectory(prefix='art-term-wheel-') as directory:
        root = Path(directory)
        home, fixture = root / 'home', root / 'fixture'
        home.mkdir(mode=0o700)
        fixture.mkdir(mode=0o700)
        runtime, app = Runtime(fixture), None
        try:
            app = Terminal(args.binary.resolve(), home, runtime.ca)
            app.start()
            app.click_text('New Host')
            app.wait_text('HTTPS Runtime URL')
            app.send('Wheel fixture\t' + runtime.origin + '\t' + runtime.credential)
            app.send(b'\x13')
            app.wait(lambda: '1 Hosts' in app.text() and 'HTTPS Runtime URL' not in app.text(), 'saved Host')
            app.shells()
            app.click_text('New Shell', region=(0, 1, 40, 24))
            app.wait(lambda: len(runtime.work()) == 1, 'work created')
            first = next(iter(runtime.work()))
            app.wait(lambda: runtime.work()[first]['bootstrap'], 'bootstrap')
            app.wait_text('Live')

            def received(name, start):
                return b''.join(data for tab, data in runtime.frames[start:] if tab == name)

            def check(label):
                result['checks'].append(label)
                print('PASS ' + label, flush=True)

            runtime.emit(first, b'\x1b[?1049h\x1b[?1000h\x1b[?1006hMOUSE-READY')
            app.wait_text('MOUSE-READY')
            start, events = len(runtime.frames), len(runtime.snapshot()['events'])
            app.wheel(62, 8, -1)
            app.wait(lambda: b'\x1b[<64;22;6M' in received(first, start),
                     'mouse-aware CLI receives upward wheel with terminal-local coordinates', 3)
            app.wheel(62, 8, 1)
            app.wait(lambda: b'\x1b[<65;22;6M' in received(first, start), 'downward wheel reaches CLI')
            app.send(b'\x1b[<92;63;9M')
            app.wait(lambda: b'\x1b[<92;22;6M' in received(first, start), 'wheel modifiers survive')
            assert not any(e.get('action') == 'scroll' for e in runtime.snapshot()['events'][events:])
            check('Mouse-aware full-screen CLI receives up/down/modifier wheel, without host-history requests')

            start = len(runtime.frames)
            app.wheel(40, 8, -1)
            app.settle(.2)
            assert not received(first, start), 'terminal border wheel reached CLI'
            app.resize(30, 90)
            app.wait(lambda: runtime.work()[first]['resizes'][-1] == [24, 58], 'resize applied')
            app.wheel(45, 8, -1)
            app.wait(lambda: b'\x1b[<64;15;6M' in received(first, start), 'resized local coordinates')
            check('Border wheel is ignored; resize retains correct terminal-local coordinates')

            app.resize(36, 120)
            app.wait(lambda: runtime.work()[first]['resizes'][-1] == [30, 78], 'original dimensions')
            runtime.emit(first, b'\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1049lSHELL-READY')
            app.wait_text('SHELL-READY')
            runtime.append_lines(first, 0, 150)
            app.wait_text('ROW-0149')
            app.wheel(62, 8, -5)
            app.wait_text('ROW-0105')
            assert 'ROW-0149' not in app.text(), 'older viewport still displays latest row'
            app.wheel(62, 8, 5)
            app.wait_text('ROW-0149')
            app.wheel(62, 8, -1)
            app.wait_text('歷史閱讀')
            app.send('WHEEL-FOLLOW')
            app.wait(lambda: runtime.work()[first]['input'].endswith('WHEEL-FOLLOW'), 'follow then input')
            assert runtime.work()[first]['input'].count('WHEEL-FOLLOW') == 1
            check('Clearing CLI mouse mode restores actual old-row history, downward latest and exactly-once follow input')

            app.prefix('n')
            app.wait(lambda: len(runtime.work()) == 2, 'second work')
            second = next(name for name in runtime.work() if name != first)
            app.wait(lambda: runtime.work()[second]['bootstrap'], 'second bootstrap')
            app.wait_text('2:')
            runtime.append_lines(second, 200, 150)
            app.wait_text('ROW-0349')
            start = len(runtime.frames)
            app.wheel(62, 8, -5)
            app.wait_text('ROW-0305')
            assert not received(first, start), 'inactive tab received wheel input'
            check('Wheel targets the selected tab without leaking to an inactive terminal')
            app.prefix('q')
            app.finish()
            result['result'] = 'PASS'
        except Exception:
            result['result'] = 'FAIL'
            result['failure'] = traceback.format_exc()
            print(result['failure'], flush=True)
            if app:
                app.save(output, 'failure')
        finally:
            if app:
                app.close()
            runtime.close()
    (output / 'result.json').write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    return int(result['result'] != 'PASS')


if __name__ == '__main__':
    raise SystemExit(main())
