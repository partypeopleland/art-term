#!/usr/bin/env python3
"""Public CLI regression using owned tmux, real bundle, and optional pinned Runtime."""
import argparse
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import pty
import secrets
import select
import shlex
import re
import shutil
import signal
import struct
import subprocess
import tempfile
import termios
import time

REPO = Path(__file__).resolve().parents[2]
ASSETS = REPO / 'scripts/openab-runtime'


def require(value, message):
    if not value:
        raise AssertionError(message)


class Reader:
    def __init__(self, stream):
        self.fd = stream.fileno() if hasattr(stream, 'fileno') else stream
        self.buffer = b''

    def frame(self, timeout=10):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            while b'\n' in self.buffer:
                line, self.buffer = self.buffer.split(b'\n', 1)
                try:
                    value = json.loads(line.rstrip(b'\r'))
                    if isinstance(value, dict) and value.get('v') == 1:
                        return value
                except (ValueError, UnicodeError):
                    pass
            if select.select([self.fd], [], [], max(0, deadline - time.monotonic()))[0]:
                data = os.read(self.fd, 65536)
                require(data, 'helper exited before reply')
                self.buffer += data
                require(len(self.buffer) < 300000, 'bounded frame')
        raise AssertionError('helper reply deadline')


def run_local(bundle, result, lease=False, r5=False, aba=False, bounded=False, paged=False):
    with tempfile.TemporaryDirectory(prefix='art-term-tmux-cli-') as folder:
        home = Path(folder)
        env = dict(os.environ, HOME=str(home), TERM='xterm-256color')
        key, nonce = secrets.token_hex(12), secrets.token_hex(16)
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
        processes = []
        state = home / ('.local/share/art-term/tmux-history-v2' if paged else '.local/share/art-term/tmux-history') / key
        actual_tmux = str(REPO/'target/tmux-lazy-proof/tmux-3.6a/tmux') if paged else shutil.which('tmux')
        wrapper = ASSETS / ('tmux-v2/tmux-shell.py' if paged else 'tmux-shell.py')
        private_binary = home / '.local/share/art-term/openab-pty/tmux-v2/tmux'
        if paged:
            private_binary.parent.mkdir(parents=True)
            private_binary.symlink_to(actual_tmux)
        try:
            work = subprocess.Popen(['python3', str(wrapper), key], env=env,
                                    stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
            processes.append(work)
            deadline = time.monotonic() + 10
            while not (state / 'tmux.sock').exists():
                require(work.poll() is None, 'public work CLI failed before socket')
                require(time.monotonic() < deadline, 'work socket deadline')
                time.sleep(.02)
            tmux = lambda *args: subprocess.check_output([actual_tmux, '-S', str(state / 'tmux.sock'), *args], timeout=3)
            deadline = time.monotonic() + 5
            while True:
                try:
                    tmux('display-message', '-p', '-t', 'work:0.0', '#{pane_id}')
                    break
                except subprocess.CalledProcessError:
                    require(time.monotonic() < deadline, 'pane startup deadline')
                    time.sleep(.02)
            # Old clients get only raw shell VT, never helper metadata.
            os.write(master, b"printf '\\033[34m'; for i in $(seq 1 1000); do printf 'ROW-%04d \\347\\225\\214\\n' \"$i\"; done\n")
            deadline = time.monotonic() + 5
            while b'ROW-1000' not in tmux('capture-pane', '-p', '-J', '-S', '-', '-t', 'work:0.0'):
                require(time.monotonic() < deadline, 'producer deadline')
                time.sleep(.02)
            marker = home / 'inject-aba'
            capture_log = home / 'capture-observer.jsonl'
            if bounded:
                shim = home / 'capture-shim'; shim.mkdir()
                script = """#!/usr/bin/python3
import json,subprocess,sys,shlex
data=sys.stdin.buffer.read() if 'source-file' in sys.argv else None
p=subprocess.run([REAL,*sys.argv[1:]],input=data,capture_output=True)
args=sys.argv[1:]
if data:
 args=[]
 for line in data.decode().splitlines():args.extend([*shlex.split(line),';'])
if 'capture-pane' in args:
 with open(LOG,'a') as f:f.write(json.dumps({'args':args,'bytes':len(p.stdout)})+'\\n')
sys.stdout.buffer.write(p.stdout);sys.stderr.buffer.write(p.stderr);sys.exit(p.returncode)
""".replace('REAL', repr(actual_tmux)).replace('LOG', repr(str(capture_log)))
                (shim/'tmux').write_text(script); (shim/'tmux').chmod(0o700)
                env['PATH'] = str(shim) + ':' + env['PATH']
                if paged:
                    private_binary.unlink(); private_binary.symlink_to(shim/'tmux')
            if aba:
                marker.touch()
                shim = home / 'shim'; shim.mkdir()
                script = """#!/usr/bin/python3
import pathlib,subprocess,sys
real=REAL
args=sys.argv[1:]
active=pathlib.Path(MARKER).exists() and ('copy-mode' in args or 'capture-pane' in args)
if active:subprocess.run([real,*args[:2],'resize-window','-t','work:0','-x','40','-y','24'],check=True,stdout=subprocess.DEVNULL)
try:
 p=subprocess.run([real,*args],capture_output=True)
finally:
 if active:subprocess.run([real,*args[:2],'resize-window','-t','work:0','-x','80','-y','24'],check=True,stdout=subprocess.DEVNULL)
sys.stdout.buffer.write(p.stdout);sys.stderr.buffer.write(p.stderr);sys.exit(p.returncode)
""".replace('REAL',repr(actual_tmux)).replace('MARKER',repr(str(marker)))
                (shim/'tmux').write_text(script);(shim/'tmux').chmod(0o700)
                env['PATH']=str(shim)+':'+env['PATH']
                if paged:
                    private_binary.unlink(); private_binary.symlink_to(shim/'tmux')
            helper = subprocess.Popen(['node', str(bundle), key, nonce], env=env,
                                      stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            processes.append(helper)
            reader = Reader(helper.stdout)
            ready = reader.frame()
            require(ready['type'] == 'ready' and ready['nonce'] == nonce, 'identity ready')
            request_id = 0
            def request(op, fragment=False, **fields):
                nonlocal request_id
                request_id += 1
                prior_captures = len(capture_log.read_text().splitlines()) if capture_log.exists() else 0
                value = {'v': 1, 'nonce': nonce, 'requestId': request_id, 'op': op, **fields}
                encoded = json.dumps(value).encode() + b'\n'
                if fragment:
                    midpoint = len(encoded) // 2
                    helper.stdin.write(encoded[:midpoint])
                    helper.stdin.flush()
                    time.sleep(.02)
                    helper.stdin.write(encoded[midpoint:])
                else:
                    helper.stdin.write(encoded)
                helper.stdin.flush()
                reply = reader.frame()
                require(reply['requestId'] == request_id and reply['nonce'] == nonce, 'request identity')
                if paged:
                    windows = tmux('list-windows', '-t', 'work', '-F', '#{@art_term_reader}').decode().splitlines()
                    require(sum(bool(tag) for tag in windows) <= 2, 'more than two native reader snapshots')
                    require(tmux('display-message', '-p', '-t', 'work:0.0', '#{pane_in_mode}').strip() == b'0', 'work entered copy mode')
                if bounded and capture_log.exists():
                    captures = [json.loads(line) for line in capture_log.read_text().splitlines()]
                    result['observedCaptures'] = captures
                    new_captures = captures[prior_captures:]
                    require(not new_captures or op in ('viewport', 'older'), 'ACK/keep/close fetched history')
                    maximum = (reply.get('rows', 500) if op == 'viewport' else fields.get('limit', 0)) + 1
                    for capture in new_captures:
                        transferred = 0
                        all_args = capture['args']
                        for i, arg in enumerate(all_args):
                            if arg != 'capture-pane': continue
                            end_command = all_args.index(';', i) if ';' in all_args[i:] else len(all_args)
                            args = all_args[i:end_command]
                            require('-S' in args and '-E' in args, 'Host fetched complete history before returning a page')
                            start, end = args[args.index('-S')+1], args[args.index('-E')+1]
                            require(start.lstrip('-').isdigit() and end.lstrip('-').isdigit(), 'capture must use numeric page bounds')
                            span = int(end)-int(start)+1
                            require(0 < span <= maximum, 'capture exceeded the requested page plus one guard')
                            if '-p' in args: transferred += span
                        require(transferred <= maximum, 'one command transferred more than the requested page plus guard')
                return reply
            if aba:
                response=request('viewport')
                require(response.get('kind')=='busy' or (response.get('type')=='page' and response['columns']==40),
                        'ABA resize: captured width40 was decoded and published as width80')
                marker.unlink()
                result['abaResizeConsistentOrBusy']=True
            first = request('viewport', fragment=True)
            helper.stdin.write(json.dumps({'v': 1, 'nonce': nonce, 'requestId': request_id, 'op': 'keep', 'epoch': first['epoch']}).encode() + b'\n')
            helper.stdin.flush()
            require(reader.frame()['kind'] == 'protocol', 'request ID reuse rejected')
            require(first['type'] == 'page' and len(first['records']) <= 24 and first['hasOlder'], 'viewport only')
            epoch = first['epoch']
            require(request('ack', epoch=epoch)['type'] == 'ack', 'display ACK')
            require(request('older', epoch=epoch + 100, before=first['start'], limit=256)['kind'] == 'expired', 'wrong epoch rejected')
            require(request('older', epoch=epoch, before=first['rowCount'] + 1, limit=256)['kind'] == 'protocol', 'before bounds rejected')
            require(request('keep', epoch=epoch, nonce='0' * 32)['kind'] == 'protocol', 'wrong nonce rejected')
            old = request('older', epoch=epoch, before=first['start'], limit=256)
            require(len(old['records']) == 256 and old['end'] == first['start'], 'explicit older page')
            complete = list(first['records'])
            before = first['start']
            while before:
                chunk = request('older', epoch=epoch, before=before, limit=256)
                complete = chunk['records'] + complete
                before = chunk['start']
            ids = re.findall(r'ROW-(\d{4})', ''.join(row['text'] for row in complete))
            require(ids == [f'{i:04}' for i in range(1, 1001)], 'all source rows exactly once')
            require(all(any(style[2] == 0x569cd6 for style in row['styles']) for row in old['records']), 'inherited color')
            os.write(master, b"for i in $(seq 1001 2000); do printf 'ROW-%04d\\n' \"$i\"; done\n")
            deadline = time.monotonic() + 5
            while b'ROW-2000' not in tmux('capture-pane', '-p', '-J', '-S', '-', '-t', 'work:0.0'):
                require(time.monotonic() < deadline, 'second producer deadline')
                time.sleep(.02)
            time.sleep(.51)
            candidate = request('viewport')
            require(candidate['epoch'] != epoch, 'new viewport epoch')
            repeated = request('viewport')
            require(repeated == {**candidate, 'requestId': repeated['requestId']}, 'inflight candidate pinned')
            require(request('older', epoch=epoch, before=first['start'], limit=256)['records'] == old['records'], 'old epoch immutable')
            os.write(master, b"for i in $(seq 2001 8000); do printf 'ROW-%04d\\n' \"$i\"; done\n")
            deadline = time.monotonic() + 5
            while b'ROW-8000' not in tmux('capture-pane', '-p', '-J', '-S', '-', '-t', 'work:0.0'):
                require(time.monotonic() < deadline, 'history cap producer deadline')
                time.sleep(.02)
            require(b'ROW-0001' not in tmux('capture-pane', '-p', '-J', '-S', '-', '-t', 'work:0.0'), 'actual history limit evicts original rows')
            require(request('older', epoch=epoch, before=first['start'], limit=256)['records'] == old['records'], 'history-limit snapshot immutable')
            tmux('clear-history', '-t', 'work:0.0')
            tmux('resize-window', '-t', 'work:0', '-x', '40', '-y', '12')
            require(request('older', epoch=epoch, before=first['start'], limit=256)['records'] == old['records'], 'clear resize immutable')
            require(request('older', epoch=epoch, before=first['start'], limit=257)['kind'] == 'protocol', 'limit invalid')
            require(request('ack', epoch=candidate['epoch'])['type'] == 'ack', 'candidate displayed')
            require(request('keep', epoch=epoch)['kind'] == 'expired', 'old epoch released after display')
            require(request('ack', epoch=epoch)['kind'] == 'expired', 'late ACK rejected')
            os.write(master, b"for i in $(seq 1 6000); do printf 'MAX-%04d\\n' \"$i\"; done\n")
            deadline = time.monotonic() + 5
            while b'MAX-6000' not in tmux('capture-pane', '-p', '-J', '-S', '-', '-t', 'work:0.0'):
                require(time.monotonic() < deadline, 'full snapshot producer deadline')
                time.sleep(.02)
            time.sleep(.51)
            maximum = request('viewport')
            require(maximum['type'] == 'page' and 4500 < maximum['rowCount'] <= 5024, 'full bounded snapshot decoded')
            result['fullSnapshotRows'] = maximum['rowCount']
            if r5:
                require(request('ack', epoch=maximum['epoch'])['type'] == 'ack', 'maximum displayed')
                tmux('resize-window', '-t', 'work:0', '-x', '80', '-y', '300')
                history_size = int(tmux('display-message', '-p', '-t', 'work:0.0', '#{history_size}'))
                count = max(1, 4900 - history_size)
                command = f"for i in $(seq 1 {count}); do printf 'TALL-%04d\\n' \"$i\"; done\n"
                os.write(master, command.encode())
                deadline = time.monotonic() + 5
                marker = f'TALL-{count:04}'.encode()
                while marker not in tmux('capture-pane', '-p', '-J', '-S', '-', '-t', 'work:0.0'):
                    require(time.monotonic() < deadline, 'tall producer deadline')
                    time.sleep(.02)
                time.sleep(.51)
                tall = request('viewport')
                require(tall['type'] == 'page', 'r5 >5024 snapshot accepted')
                require(tall['rows'] == 300 and len(tall['records']) == 300 and 5024 < tall['rowCount'] <= 6000, 'r5 tall viewport 300 with >5024 rows')
                require(request('older', epoch=tall['epoch'], before=tall['start'], limit=256)['end'] == tall['start'], 'r5 older bound')
                require(request('older', epoch=tall['epoch'], before=tall['start'], limit=300)['kind'] == 'protocol', 'r5 older still <=256')
                require(request('ack', epoch=tall['epoch'])['type'] == 'ack', 'r5 tall display ACK')
                result['tallViewportRows'] = len(tall['records'])
                result['tallSnapshotRows'] = tall['rowCount']
                # Keep decoder below its 1280KiB budget but make one viewport JSON
                # larger than 256KiB via many distinct real SGR style runs.
                (home / 'style-pressure.py').write_text("import sys\nfor y in range(300):\n sys.stdout.write(''.join('\\x1b[38;2;%d;1;2mX'%x for x in range(60))+'\\n')\nsys.stdout.flush()\n")
                tmux('clear-history', '-t', 'work:0.0')
                os.write(master, b"printf '\\033[2J\\033[H'; python3 \"$HOME/style-pressure.py\"\n")
                deadline = time.monotonic() + 5
                while tmux('capture-pane', '-p', '-e', '-J', '-S', '-', '-t', 'work:0.0').count(b'[38;2;59;1;2m') < 290:
                    require(time.monotonic() < deadline, 'style producer deadline')
                    time.sleep(.02)
                time.sleep(.51)
                overcap = request('viewport')
                require(overcap['type'] == 'error' and overcap['kind'] == 'limit', 'encoded viewport cap explicit')
                # A candidate that never reached the client must not poison pinning.
                tmux('clear-history', '-t', 'work:0.0')
                os.write(master, b"printf '\\033[0m\\033[2J\\033[HFRAME-CAP-RECOVERED\\n'\n")
                deadline = time.monotonic() + 5
                while b'FRAME-CAP-RECOVERED' not in tmux('capture-pane', '-p', '-J', '-S', '-', '-t', 'work:0.0'):
                    require(time.monotonic() < deadline, 'cap recovery producer deadline')
                    time.sleep(.02)
                time.sleep(.51)
                recovered = request('viewport')
                require(recovered['type'] == 'page' and any('FRAME-CAP-RECOVERED' in r['text'] for r in recovered['records']), 'unsent overcap candidate does not stay pinned')
                result['encodedFrameCap'] = True
                result['overcapCandidateRecovery'] = True
            status = Path(f'/proc/{helper.pid}/status').read_text()
            result['builtWorkerRssKiB'] = int(re.search(r'VmRSS:\s+(\d+)', status)[1])
            if lease:
                # Real wall-clock boundary: keep at 65s extends an original 120s lease.
                time.sleep(59)
                time.sleep(6)
                require(request('keep', epoch=candidate['epoch'])['type'] == 'ack', 'lease keep')
                time.sleep(59)
                time.sleep(6)
                require(request('older', epoch=candidate['epoch'], before=candidate['start'], limit=256)['type'] == 'page', 'sliding lease survives 130s total')
                time.sleep(59)
                time.sleep(59)
                time.sleep(3)
                require(request('keep', epoch=candidate['epoch'])['kind'] == 'expired', 'idle lease expires at 120s')
                fresh = request('viewport')
                require(fresh['type'] == 'page' and fresh['epoch'] != candidate['epoch'], 'explicit latest recovers expired epoch')
                result['realSlidingLeaseAndExpiry'] = True
            require(request('close')['type'] == 'ack', 'close')
            helper.wait(timeout=3)
            if paged:
                require(tmux('list-windows', '-t', 'work', '-F', '#{window_index}').splitlines() == [b'0'], 'close leaked reader windows or killed work')
                killed_nonce = secrets.token_hex(16)
                killed = subprocess.Popen(['node', str(bundle), key, killed_nonce], env=env,
                    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                processes.append(killed)
                killed_reader = Reader(killed.stdout)
                require(killed_reader.frame()['type'] == 'ready', 'replacement helper ready')
                killed.stdin.write(json.dumps({'v':1,'nonce':killed_nonce,'requestId':1,'op':'viewport'}).encode()+b'\n'); killed.stdin.flush()
                require(killed_reader.frame()['type'] == 'page', 'replacement owns real reader')
                killed.kill(); killed.wait(timeout=3)
            overflow = subprocess.Popen(['node', str(bundle), key, secrets.token_hex(16)], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            processes.append(overflow)
            overflow_reader = Reader(overflow.stdout)
            require(overflow_reader.frame()['type'] == 'ready', 'overflow worker ready')
            if paged:
                require(tmux('list-windows', '-t', 'work', '-F', '#{window_index}').splitlines() == [b'0'], 'startup failed to reclaim confirmed-dead reader')
                result['nativeReaderCleanupAfterKill'] = True
            overflow.stdin.write(b'{' + b'x' * 9000)
            overflow.stdin.flush()
            require(overflow_reader.frame()['kind'] == 'protocol', 'oversized fragment rejected')
            require(overflow.wait(timeout=3) != 0, 'oversized fragment closes bounded worker')
            # A live namespace may never be reused or overwritten.
            duplicate = subprocess.run(['python3', str(wrapper), key], env=env,
                                       input=b'', capture_output=True, timeout=3)
            require(duplicate.returncode != 0, 'reject duplicate work key')
            # A worker launched by another real parent cannot read the live work pane.
            foreign = subprocess.run(['python3', '-c', 'import subprocess,sys; p=subprocess.run(sys.argv[1:]); sys.exit(p.returncode)', 'node', str(bundle), key, secrets.token_hex(16)], env=env, input=b'', capture_output=True, timeout=5)
            require(foreign.returncode != 0 and b'"type":"ready"' not in foreign.stdout, 'foreign real parent rejected')
            # Wrong Runtime ancestry fails before ready, even with a real live pane.
            manifest = state / 'manifest.json'
            value = json.loads(manifest.read_text())
            value['runtimePid'] = 1
            manifest.write_text(json.dumps(value))
            rejected = subprocess.run(['node', str(bundle), key, secrets.token_hex(16)], env=env,
                                      input=b'', capture_output=True, timeout=5)
            frames = [json.loads(line) for line in rejected.stdout.splitlines()]
            require(rejected.returncode != 0 and all(f['type'] != 'ready' for f in frames), 'cross Runtime rejected')
            result.update(localPassed=True, initialRows=len(first['records']), olderRows=len(old['records']),
                          fullRows=first['rowCount'], immutableEpoch=True, pinnedCandidate=True, parentProof=True,
                          all1000ExactlyOnce=True, historyLimitEviction=True, wrongEpoch=True, lateAck=True, nonceValidation=True, requestOverflow=True, foreignParentRejected=True, fragmentedRequest=True, reusedRequestRejected=True)
        finally:
            for process in reversed(processes):
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=3)
            if (state / 'tmux.sock').exists():
                subprocess.run([actual_tmux, '-S', str(state / 'tmux.sock'), 'kill-server'], capture_output=True, timeout=3)
            os.close(master)
            os.close(slave)
            result['localCleanup'] = all(p.poll() is not None for p in processes)


def run_runtime(bundle, binary, result, paged=False):
    spec = importlib.util.spec_from_file_location('official_harness', REPO / 'scripts/linux-regression/run.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    # AF_UNIX sockets are limited to 108 bytes, so keep owned fixture HOME short.
    root = Path(tempfile.mkdtemp(prefix='th-'))
    harness = module.Harness.__new__(module.Harness)
    sockets = []
    states = []
    try:
        harness.__init__(root, binary)
        support = harness.helper / 'tmux-v2' if paged else harness.helper
        support.mkdir(exist_ok=True)
        shutil.copyfile(ASSETS / ('tmux-v2/tmux-shell.py' if paged else 'tmux-shell.py'), support / 'tmux-shell.py')
        shutil.copyfile(bundle, support / 'tmux-history.cjs')
        actual_tmux = str(REPO/'target/tmux-lazy-proof/tmux-3.6a/tmux') if paged else 'tmux'
        if paged:
            (support/'tmux').symlink_to(actual_tmux)
        (harness.helper/'node').symlink_to(REPO/'target/android-tools/node-v22.23.3-linux-x64/bin/node')
        key, nonce = secrets.token_hex(12), secrets.token_hex(16)
        work_name = ('th2-' if paged else 'th-') + key
        status, grant = harness.api('POST', '/admin/sessions', {'name': work_name})
        require(status == 200, 'work create')
        harness.owned.add(work_name)
        work = harness.attach(work_name, grant)
        sockets.append(work)
        work.control(type='resize', rows=24, cols=80)
        version_dir = 'tmux-v2/' if paged else ''
        work.send(2, ('exec python3 "$HOME/.local/share/art-term/openab-pty/'+version_dir+'tmux-shell.py" ' + key + '\n').encode())
        state = harness.home / ('.local/share/art-term/tmux-history-v2' if paged else '.local/share/art-term/tmux-history') / key
        states.append(state)
        deadline = time.monotonic() + 10
        while not (state / 'tmux.sock').exists():
            require(time.monotonic() < deadline, 'Runtime work startup deadline')
            time.sleep(.02)
        # Decode the same raw work stream from an ordinary old-client attach.
        work.send(2, b"printf '\\033[34m'; for i in $(seq 1 1000); do printf 'RUNTIME-%04d\\n' \"$i\"; done\n")
        raw = work.collect(b'\x1b')
        deadline = time.monotonic() + 5
        while b'RUNTIME-1000' not in subprocess.check_output([actual_tmux, '-S', str(state / 'tmux.sock'), 'capture-pane', '-p', '-J', '-S', '-', '-t', 'work:0.0'], timeout=3):
            require(time.monotonic() < deadline, 'Runtime producer deadline')
            time.sleep(.02)
        require(b'"type":"ready"' not in raw and b'"records":' not in raw, 'raw work compatibility')
        manifest = json.loads((state / 'manifest.json').read_text())
        require(manifest['runtimePid'] == harness.process.pid, 'real Runtime direct parent')
        name, helper_grant = harness.create('mcshot-')
        helper = harness.attach(name, helper_grant)
        sockets.append(helper)
        helper.send(2, ('exec "$HOME/.local/share/art-term/openab-pty/node" "$HOME/.local/share/art-term/openab-pty/'+version_dir+'tmux-history.cjs" ' + key + ' ' + nonce + '\n').encode())
        ready_raw = helper.collect(b'"type":"ready"')
        result['runtimeReadyWire'] = ready_raw.decode('utf8', 'replace')
        stream = ready_raw
        def frame(request_id):
            nonlocal stream
            deadline = time.monotonic() + 10
            while True:
                while b'\n' in stream:
                    line, stream = stream.split(b'\n', 1)
                    try:
                        value = json.loads(line.rstrip(b'\r'))
                        if value.get('nonce') == nonce and value.get('requestId') == request_id:
                            return value
                    except (ValueError, AttributeError, UnicodeError):
                        pass
                require(time.monotonic() < deadline, 'Runtime frame deadline')
                opcode, data = helper.recv(deadline - time.monotonic())
                if opcode == 2:
                    stream += data
                    require(len(stream) < 300000, 'Runtime frame bound')
                elif opcode == 1:
                    require(json.loads(data).get('type') != 'gap', 'Runtime helper no Gap')
                else:
                    require(opcode != 8, 'Runtime helper premature close')
        ready = frame(0)
        require(ready['type'] == 'ready', 'real Runtime worker ready')
        helper.send(2, json.dumps({'v': 1, 'nonce': nonce, 'requestId': 1, 'op': 'viewport'}).encode() + b'\n')
        page = frame(1)
        require(page['type'] == 'page' and len(page['records']) <= 24 and page['hasOlder'], 'Runtime viewport only')
        helper.send(2, json.dumps({'v': 1, 'nonce': nonce, 'requestId': 2, 'op': 'older',
                                  'epoch': page['epoch'], 'before': page['start'], 'limit': 256}).encode() + b'\n')
        older = frame(2)
        require(older['type'] == 'page' and len(older['records']) == 256, 'Runtime older explicit')
        if paged:
            harness.delete(name)
            helper.close()
            name, helper_grant = harness.create('mcshot-')
            helper = harness.attach(name, helper_grant); sockets.append(helper)
            helper.send(2, ('exec "$HOME/.local/share/art-term/openab-pty/node" "$HOME/.local/share/art-term/openab-pty/'+version_dir+'tmux-history.cjs" ' + key + ' ' + nonce + '\n').encode())
            stream = helper.collect(b'"v":1')
            replacement = frame(0)
            require(replacement['type'] == 'ready', 'Runtime replacement helper: '+str(replacement))
            helper.send(2, json.dumps({'v':1,'nonce':nonce,'requestId':1,'op':'viewport'}).encode()+b'\n')
            replacement_page = frame(1)
            require(replacement_page['type']=='page', 'Runtime replacement viewport: '+str(replacement_page))
            result['runtimeHelperReplacement'] = True
        result.update(officialRuntimePassed=True, runtimeParentProof=True, rawOldClientCompatible=True,
                      runtimeInitialRows=len(page['records']), runtimeOlderRows=len(older['records']))
    finally:
        for ws in sockets:
            ws.close()
        try:
            harness.close()
        finally:
            for state in states:
                if (state / 'tmux.sock').exists():
                    subprocess.run([actual_tmux, '-S', str(state / 'tmux.sock'), 'kill-server'], capture_output=True, timeout=3)
        result['runtimeCleanup'] = True
        shutil.rmtree(root)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--bundle', type=Path, default=REPO / 'target/tmux-history/tmux-history.cjs')
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--aba', action='store_true')
    parser.add_argument('--r5', action='store_true', help='test 300-row viewport and encoded frame limit')
    parser.add_argument('--lease', action='store_true', help='also test real 120s sliding lease/expiry (about 251s)')
    parser.add_argument('--bounded', action='store_true', help='observe actual helper capture ranges; reject complete-history reads')
    parser.add_argument('--paged', action='store_true', help='exercise private tmux v2 work and worker')
    args = parser.parse_args()
    result = {'passed': False}
    try:
        run_local(args.bundle.resolve(), result, args.lease, args.r5, args.aba, args.bounded, args.paged)
        if args.binary:
            run_runtime(args.bundle.resolve(), args.binary.resolve(), result, args.paged)
        result['passed'] = True
    finally:
        raw = json.dumps(result, indent=2) + '\n'
        print(raw)
        if args.output:
            args.output.write_text(raw)


if __name__ == '__main__':
    main()
