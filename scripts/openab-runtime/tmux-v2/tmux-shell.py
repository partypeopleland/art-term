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
    # Older Runtime services may still inherit a retired fixed desktop grant.
    # Never pass those keys to new tmux servers or work Shells.
    for name in list(os.environ):
        if name.startswith(('MY_CLI_DESKTOP_', 'ART_TERM_DESKTOP_')) or name == 'HANDS_MCP_TOKEN':
            del os.environ[name]
    os.umask(0o077)
    base = Path.home() / '.local/share/art-term/tmux-history-v2'
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
    # This exclusive state directory creates a fresh private server: its
    # original work pane is %0. Reader snapshots use later pane IDs. End only
    # the owning session when %0 exits, even if reader windows remain alive.
    config.write_text('set -g history-limit 5000\nset -g status off\nset -g mouse off\nset -g destroy-unattached on\n'
                      'set-hook -g pane-exited \'if-shell -F "#{==:#{hook_pane},%0}" "kill-session -t =work"\'\n')
    manifest = {'v': 2, 'key': key, 'clientPid': os.getpid(), 'clientStart': client_start,
                'runtimePid': runtime_pid, 'runtimeStart': runtime_start}
    (state / 'manifest.json').write_text(json.dumps(manifest) + '\n')
    binary = Path.home() / '.local/share/art-term/openab-pty/tmux-v2/tmux'
    tools = Path.home() / '.local/share/art-term/tools'
    supervisor = tools / 'instance_bridge.py'
    command = ['python3', str(supervisor), 'shell', '--binding', 'th2-' + key] if all((tools / name).is_file() for name in ('instance_bridge.py', 'instance-shell.py', 'instance-shell.bash')) else ['bash']
    os.execv(str(binary), [str(binary), '-S', str(state / 'tmux.sock'), '-f', str(config),
                       'new-session', '-s', 'work', *command])


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError):
        print('Host history Shell unavailable', file=sys.stderr)
        sys.exit(1)
