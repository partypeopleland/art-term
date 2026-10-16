#!/usr/bin/env python3
"""Create one private tmux work Shell. Never changes existing Shells or user tmux."""
import json
import os
from pathlib import Path
import re
import stat
import sys


def process(pid):
    path = Path(f'/proc/{pid}')
    fields = (path / 'stat').read_text().rsplit(')', 1)[1].split()
    if path.stat().st_uid != os.getuid():
        raise ValueError('owner')
    return int(fields[1]), fields[19]


def main():
    if len(sys.argv) != 2 or not re.fullmatch('[0-9a-f]{24}', sys.argv[1]):
        raise ValueError('key')
    key = sys.argv[1]
    os.umask(0o077)
    base = Path.home() / '.local/share/art-term/tmux-history'
    # Refuse symlink traversal even in owner-managed parent directories.
    for parent in reversed([base, *base.parents]):
        if parent.is_symlink():
            raise ValueError('symlink')
    base.mkdir(parents=True, exist_ok=True, mode=0o700)
    info = base.lstat()
    if info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o700:
        raise ValueError('directory')
    state = base / key
    state.mkdir(mode=0o700)  # exclusive: never overwrite even stale namespaces
    runtime_pid, client_start = process(os.getpid())
    _, runtime_start = process(runtime_pid)
    config = state / 'tmux.conf'
    config.write_text('set -g history-limit 5000\nset -g status off\nset -g mouse off\nset -g destroy-unattached on\n')
    manifest = {'v': 1, 'key': key, 'clientPid': os.getpid(), 'clientStart': client_start,
                'runtimePid': runtime_pid, 'runtimeStart': runtime_start}
    (state / 'manifest.json').write_text(json.dumps(manifest) + '\n')
    os.execvp('tmux', ['tmux', '-S', str(state / 'tmux.sock'), '-f', str(config),
                       'new-session', '-s', 'work', 'bash'])


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError):
        print('Host history Shell unavailable', file=sys.stderr)
        sys.exit(1)
