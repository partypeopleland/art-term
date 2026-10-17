#!/usr/bin/env python3
"""Real private tmux/PTTY/native mouse proof; no simulated terminal renderer."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import pty
import secrets
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time
import threading


def require(value, message):
    if not value:
        raise AssertionError(message)


def main():
    parser = argparse.ArgumentParser()
    for name in ('bundle', 'wrapper', 'tmux', 'node', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--overlap', action='store_true')
    args = parser.parse_args()
    output = args.output.resolve()
    require(output.is_relative_to(Path('/home/art/projects/my-cli/target')), 'test output outside project target')
    output.mkdir(parents=True, exist_ok=True)
    temporary = tempfile.TemporaryDirectory(prefix='owned-', dir=output)
    directory = temporary.name
    result = {'passed': False, 'realTmux': True, 'checks': {}}
    processes = []
    master = slave = None
    tmux = None
    try:
        home = Path(directory)
        key = secrets.token_hex(12)
        # Relative HOME keeps the Unix socket name under sun_path's 108-byte cap.
        env = dict(os.environ, HOME='.', TERM='xterm-256color', LANG='C.UTF-8')
        private = home / '.local/share/art-term/openab-pty/tmux-v2/tmux'
        private.parent.mkdir(parents=True)
        private.symlink_to(args.tmux.resolve())
        state = home / '.local/share/art-term/tmux-history-v2' / key
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
        work = subprocess.Popen(['python3', str(args.wrapper), key], env=env, cwd=home,
                                stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
        processes.append(work)

        def command(*values):
            return subprocess.check_output([str(args.tmux), '-S', str(state.relative_to(home) / 'tmux.sock'), *values], cwd=home, stderr=subprocess.DEVNULL, timeout=3).decode().strip()
        tmux = command

        def wait(predicate, seconds=5):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                if predicate():
                    return
                if select.select([master], [], [], .01)[0]:
                    os.read(master, 65536)
            raise AssertionError('native tmux state deadline')

        wait(lambda: (state / 'tmux.sock').exists())
        wait(lambda: bool(command('display-message', '-p', '-t', 'work:0.0', '#{pane_id}')))
        os.write(master, b"for i in $(seq 1 96); do printf 'ROW-%04d\\n' \"$i\"; done\n")
        wait(lambda: 'ROW-0096' in command('capture-pane', '-p', '-t', 'work:0.0'))
        require(command('show-options', '-gqv', 'mouse') == 'off', 'baseline mouse changed')
        original_wheel = command('list-keys', '-T', 'root', 'WheelUpPane')

        if args.overlap:
            private.unlink()
            shim = """#!/usr/bin/env python3
import os,sys,subprocess,time
from pathlib import Path
real=__TMUX__
gate=os.environ.get('ART_TERM_TEST_GATE')
if gate and 'show-options' in sys.argv:
    marker=Path(gate+'.captured')
    try:
        fd=os.open(marker,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600);os.close(fd)
    except FileExistsError:
        os.execv(real,[real,*sys.argv[1:]])
    done=subprocess.run([real,*sys.argv[1:]],capture_output=True,timeout=1)
    Path(gate+'.parent').write_text(str(os.getppid()))
    Path(gate+'.ready').touch()
    until=time.monotonic()+1.5
    while not Path(gate+'.release').exists():
        if time.monotonic()>until: sys.exit(90)
        time.sleep(.005)
    sys.stdout.buffer.write(done.stdout);sys.exit(done.returncode)
os.execv(real,[real,*sys.argv[1:]])
""".replace('__TMUX__', repr(str(args.tmux.resolve())))
            private.write_text(shim); private.chmod(0o700)

        def helper(extra=None):
            nonce = secrets.token_hex(16)
            process = subprocess.Popen([str(args.node), str(args.bundle), key, nonce], env=dict(env, **(extra or {})), cwd=home,
                                       stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            processes.append(process)
            buffer = bytearray()
            sequence = 0

            def frame():
                deadline = time.monotonic() + 5
                while time.monotonic() < deadline:
                    if b'\n' in buffer:
                        raw, _, rest = buffer.partition(b'\n'); buffer[:] = rest
                        if raw.strip():
                            return json.loads(raw)
                    if select.select([process.stdout], [], [], .05)[0]:
                        data = os.read(process.stdout.fileno(), 65536)
                        require(data, 'native helper exited')
                        buffer.extend(data)
                raise AssertionError('native helper response deadline')

            ready = frame()
            require(ready.get('terminalMouse') is True, 'native capability absent')

            def request(op, **values):
                nonlocal sequence
                sequence += 1
                process.stdin.write((json.dumps(dict(v=1, nonce=nonce, requestId=sequence, op=op, **values)) + '\n').encode())
                process.stdin.flush()
                reply = frame()
                require(reply.get('type') == 'ack' and reply.get('op') == op, 'native control rejected: ' + str(reply.get('kind')))
                return reply
            return process, request

        first, request_first = helper()
        request_first('terminalMouse', enabled=True)
        require(command('show-options', '-gqv', 'mouse') == 'on', 'native mouse not enabled')
        os.write(master, b'\x1b[<64;10;5M')
        wait(lambda: command('display-message', '-p', '-t', 'work:0.0', '#{pane_in_mode}') == '1')
        position = int(command('display-message', '-p', '-t', 'work:0.0', '#{scroll_position}'))
        result['checks']['firstWheelMovedRows'] = position
        require(0 < position <= 5, f'one wheel did not move a small native number of rows (actual={position})')
        for _ in range(3):
            os.write(master, b'\x1b[<65;10;5M')
        wait(lambda: command('display-message', '-p', '-t', 'work:0.0', '#{pane_in_mode}') == '0')
        result['checks']['bottomReturnsLive'] = True
        started = time.monotonic()
        os.write(master, '中文字'.encode())
        wait(lambda: '中文字' in command('capture-pane', '-p', '-t', 'work:0.0'))
        result['checks']['unicodeEchoMs'] = round((time.monotonic() - started) * 1000, 2)
        started = time.monotonic()
        os.write(master, b'\x7f')
        wait(lambda: '中文字' not in command('capture-pane', '-p', '-t', 'work:0.0') and '中文' in command('capture-pane', '-p', '-t', 'work:0.0'))
        result['checks']['backspaceMs'] = round((time.monotonic() - started) * 1000, 2)
        require(result['checks']['unicodeEchoMs'] < 200 and result['checks']['backspaceMs'] < 200, 'native PTY interaction is delayed')
        os.write(master, b'\x15')
        if args.overlap:
            gate = str(home / 'race-gate')
            candidate, candidate_request = helper({'ART_TERM_TEST_GATE': gate})
            completed = {}; errors = {}
            def background(name, fn):
                def run():
                    try: fn(); completed[name] = True
                    except Exception as error: errors[name] = str(error)
                thread = threading.Thread(target=run, daemon=True); thread.start(); return thread
            contender = background('enable', lambda: candidate_request('terminalMouse', enabled=True))
            wait(lambda: Path(gate+'.ready').exists())
            def old_close():
                request_first('close'); first.wait(timeout=5)
            closer = background('close', old_close)
            time.sleep(.25)
            blocked = 'close' not in completed
            Path(gate+'.release').touch()
            contender.join(timeout=5); closer.join(timeout=5)
            require(not contender.is_alive() and not closer.is_alive() and not errors, 'overlap request failed: '+str(errors))
            candidate_request('close'); candidate.wait(timeout=5)
            restored_mouse = command('show-options', '-gqv', 'mouse')
            restored_wheel = command('list-keys', '-T', 'root', 'WheelUpPane')
            result['checks']['captureRestoreOverlap'] = {'oldCloseBlocked':blocked,'restoredMouse':restored_mouse,'exactWheel':restored_wheel==original_wheel}
            require(blocked and restored_mouse=='off' and restored_wheel==original_wheel,
                    'capture/restore race corrupted original mouse or binding: '+str(result['checks']['captureRestoreOverlap']))

            # Kill the actual Python transaction while its tmux client is gated.
            death_gate = str(home / 'death-gate')
            doomed, doomed_request = helper({'ART_TERM_TEST_GATE':death_gate})
            dying = background('doomed', lambda: doomed_request('terminalMouse', enabled=True))
            wait(lambda: Path(death_gate+'.ready').exists())
            transaction_pid = int(Path(death_gate+'.parent').read_text())
            os.kill(transaction_pid, signal.SIGKILL)
            dying.join(timeout=5)
            require(not dying.is_alive(), 'killed transaction failed to return')
            successor, successor_request = helper()
            began = time.monotonic()
            successor_request('terminalMouse', enabled=True)
            require(time.monotonic()-began<2, 'process death left mouse lock held')
            # A previously queued/failed owner must not override its successor.
            time.sleep(.15)
            require(command('show-options','-gqv','mouse')=='on', 'dead transaction changed successor state')
            doomed_request('close'); doomed.wait(timeout=5)
            require(command('show-options','-gqv','mouse')=='on', 'dead owner cleanup changed successor state')
            successor_request('close'); successor.wait(timeout=5)
            require(command('show-options','-gqv','mouse')=='off', 'kill recovery lost original mouse')
            require(command('list-keys','-T','root','WheelUpPane')==original_wheel,'kill recovery lost original binding')
            result['checks']['killedTransactionReleasesLockAndFencesLateOwner'] = True
            first, request_first = helper()
            request_first('terminalMouse', enabled=True)

        second, request_second = helper()
        request_second('terminalMouse', enabled=True)
        request_first('close'); first.wait(timeout=5)
        require(command('show-options', '-gqv', 'mouse') == 'on', 'old owner cleanup disabled new owner')
        os.write(master, b'\x1b[<64;10;5M')
        wait(lambda: command('display-message', '-p', '-t', 'work:0.0', '#{pane_in_mode}') == '1')
        request_second('close'); second.wait(timeout=5)
        require(command('show-options', '-gqv', 'mouse') == 'off', 'native mouse was not restored')
        require(command('list-keys', '-T', 'root', 'WheelUpPane') == original_wheel, 'original wheel binding was not restored exactly')
        require(command('display-message', '-p', '-t', 'work:0.0', '#{pane_in_mode}') == '0', 'owned copy-mode remained after cleanup')
        result['checks']['ownerHandoffAndMouseCopyModeRestore'] = True
        result['passed'] = True
    except Exception as error:
        result['error'] = str(error)
        raise
    finally:
        if tmux:
            try:
                tmux('kill-server')
            except Exception:
                pass
        for process in reversed(processes):
            if process.poll() is None:
                if process is processes[0]:
                    os.killpg(process.pid, signal.SIGTERM)
                else:
                    process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill(); process.wait(timeout=5)
        for descriptor in (master, slave):
            if descriptor is not None:
                os.close(descriptor)
        temporary.cleanup()
        result['cleanup'] = all(process.poll() is not None for process in processes)
        (output / 'result.json').write_text(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
