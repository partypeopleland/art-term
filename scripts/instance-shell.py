"""Owned interactive Shell supervisor; no target data is passed to its child."""
import errno
import fcntl
import os
from pathlib import Path
import pty
import select
import signal
import stat
import struct
import termios
import time
import tty


def identity(pid):
    path = Path('/proc') / str(pid)
    fields = (path/'stat').read_text().rsplit(')', 1)[1].split()
    return path.stat().st_uid, int(fields[3]), fields[19], fields[0]


def members(session):
    found = {}
    for path in Path('/proc').glob('[0-9]*'):
        try:
            value = identity(int(path.name))
            if int(path.name) != os.getpid() and value[0] == os.getuid() and value[1] == session and value[3] != 'Z':
                found[int(path.name)] = value
        except (OSError, ValueError):
            pass
    return found


def reap_session(session):
    # Each signal is fenced by UID, session and Linux process start time.
    for sig, duration in ((signal.SIGHUP, .3), (signal.SIGTERM, .5), (signal.SIGKILL, .3)):
        end = time.monotonic() + duration
        while True:
            owned = members(session)
            if not owned: return
            for pid, expected in owned.items():
                try:
                    if identity(pid) == expected: os.kill(pid, sig)
                except (OSError, ValueError): pass
            if time.monotonic() >= end: break
            time.sleep(.05)


def guard(read_fd, done_fd, session, path):
    os.setpgid(0, 0)
    signal.signal(signal.SIGHUP, signal.SIG_IGN)
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    # Socket inode supplied by parent after creation, before Shell can be used.
    record = b''
    while True:
        chunk = os.read(read_fd, 128)
        if not chunk: break
        record += chunk
        if len(record) > 128: break
    reap_session(session)
    try:
        device, inode = map(int, record.split())
        info = path.lstat()
        if info.st_uid == os.getuid() and stat.S_ISSOCK(info.st_mode) and (info.st_dev, info.st_ino) == (device, inode):
            path.unlink()
    except (OSError, ValueError): pass
    os.close(done_fd)
    os._exit(0)


def shell(binding, broker_factory, serve, directory):
    if not os.isatty(0) or not os.isatty(1): raise ValueError('Shell requires a TTY')
    rc = Path(__file__).with_name('instance-shell.bash')
    if not rc.is_file(): raise ValueError('Shell rc missing')
    path = directory(binding)/'shell.sock'
    if path.exists(): raise ValueError('Shell binding already exists')
    saved = termios.tcgetattr(0)
    gate_read, gate_write = os.pipe()
    read_fd, write_fd = os.pipe()
    done_read, done_write = os.pipe()
    child, master = pty.fork()
    if child == 0:
        os.close(gate_write); os.close(write_fd); os.close(done_read)
        # Guard membership pins this original session until cleanup completes,
        # even when the Bash session leader exits before its descendants.
        watcher = os.fork()
        if watcher == 0:
            os.close(gate_read)
            guard(read_fd, done_write, os.getsid(0), path)
        os.close(read_fd); os.close(done_write)
        if os.read(gate_read, 1) != b'1': os._exit(1)
        os.close(gate_read)
        inherited = {key:value for key,value in os.environ.items() if not key.startswith(('MY_CLI_DESKTOP_', 'ART_TERM_DESKTOP_')) and key != 'HANDS_MCP_TOKEN'}
        env = dict(inherited, ART_TERM_SCREEN_BINDING=binding,
                   ART_TERM_SCREEN_BRIDGE=str(Path(__file__).with_name('instance_bridge.py').resolve()))
        os.execvpe('bash', ['bash', '--rcfile', str(rc), '-i'], env)
    os.close(gate_read); os.close(read_fd); os.close(done_write)
    broker = broker_factory(); server = None
    old_handlers = {}
    stopped = [False]
    def stop(_sig, _frame): stopped[0] = True
    def resize(_sig=None, _frame=None):
        try:
            size = fcntl.ioctl(0, termios.TIOCGWINSZ, bytes(8))
            fcntl.ioctl(master, termios.TIOCSWINSZ, size)
        except OSError: pass
    status = None
    try:
        server, path = serve(binding, broker, shell=True)
        info = path.lstat(); os.write(write_fd, f'{info.st_dev} {info.st_ino}'.encode())
        os.write(gate_write, b'1'); os.close(gate_write); gate_write = None
        for sig in (signal.SIGHUP, signal.SIGTERM, signal.SIGINT):
            old_handlers[sig] = signal.signal(sig, stop)
        old_handlers[signal.SIGWINCH] = signal.signal(signal.SIGWINCH, resize)
        resize(); tty.setraw(0)
        for fd in (master, 0, 1): os.set_blocking(fd, False)
        incoming, outgoing = bytearray(), bytearray()
        while not stopped[0]:
            done, value = os.waitpid(child, os.WNOHANG)
            if done: status = value; break
            readers = ([0] if len(incoming) < 65536 else []) + ([master] if len(outgoing) < 65536 else [])
            writers = ([master] if incoming else []) + ([1] if outgoing else [])
            ready, writable, _ = select.select(readers, writers, [], .1)
            for fd in ready:
                try: data = os.read(fd, 16384)
                except BlockingIOError: continue
                except OSError as e:
                    if e.errno == errno.EIO: data = b''
                    else: raise
                if not data:
                    stopped[0] = True; break
                (incoming if fd == 0 else outgoing).extend(data)
            for fd in writable:
                buffer = incoming if fd == master else outgoing
                try: del buffer[:os.write(fd, buffer)]
                except BlockingIOError: pass
            if len(incoming) > 81920 or len(outgoing) > 81920: raise ValueError('TTY buffer bound')
        # EOF/EIO ends reading; queued bytes still need flushing.
        end = time.monotonic()+.3
        eof = False
        while time.monotonic() < end and (not eof or outgoing):
            ready, writable, _ = select.select([] if eof else [master], [1] if outgoing else [], [], .02)
            if ready:
                try:
                    data = os.read(master, 16384)
                    if data: outgoing.extend(data)
                    else: eof = True
                except BlockingIOError: pass
                except OSError as error:
                    if error.errno == errno.EIO: eof = True
                    else: raise
            if writable:
                try: del outgoing[:os.write(1, outgoing)]
                except BlockingIOError: pass
            if len(outgoing) > 81920: break
    finally:
        if gate_write is not None: os.close(gate_write)
        broker.close()
        # Death guard owns bounded process cleanup and inode-fenced socket removal.
        os.close(write_fd)
        if not select.select([done_read], [], [], 3)[0]:
            raise RuntimeError('Shell cleanup deadline')
        os.read(done_read, 1); os.close(done_read)
        if status is None:
            try: _, status = os.waitpid(child, 0)
            except ChildProcessError: pass
        if server: server.shutdown(); server.server_close()
        os.close(master)
        for fd in (0, 1): os.set_blocking(fd, True)
        termios.tcsetattr(0, termios.TCSANOW, saved)
        for sig, handler in old_handlers.items(): signal.signal(sig, handler)
    return os.waitstatus_to_exitcode(status) if status is not None else 1
