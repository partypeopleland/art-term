#!/usr/bin/env python3
"""Optional desktop preview launcher for an unmodified official openab-pty Host.

The App invokes this inside a dedicated temporary Shell:

    python3 "$HOME/.local/share/art-term/openab-pty/preview.py" screenshot NONCE
    python3 "$HOME/.local/share/art-term/openab-pty/preview.py" status     NONCE

NONCE is exactly 32 lowercase hex digits chosen by the App. The PTY contract is
three framed lines on stdout (preceded by a newline):

    ARTTERM_PREVIEW_BEGIN:<nonce>
    <one line of helper JSON>
    ARTTERM_PREVIEW_END:<nonce>:<exit_code>

MCP credentials are read at invocation time from a protected owner-only
KEY=VALUE file (default: desktop.env next to this launcher; a Host-managed
ART_TERM_PREVIEW_CONFIG environment variable may name another protected file).
The file is parsed, never executed. Credentials never appear in the PTY
command, its echo, output or diagnostics; helper stderr and tracebacks are
never echoed either.
"""
import json
import os
import re
import stat
import subprocess
import sys
import time
from pathlib import Path

NONCE = re.compile(r'[0-9a-f]{32}')
ENV_LINE = re.compile(r'([A-Za-z_][A-Za-z0-9_]*)=([\x20-\x7e]*)')
IDENT = re.compile(r'[A-Za-z_][A-Za-z0-9_]*')
HELPER_TIMEOUT = 20          # helper performs a few bounded MCP calls (5s each)
MAX_ENV_BYTES = 64 * 1024
MAX_OUTPUT = 12 * 1024 * 1024  # one JSON line holding an <=8 MiB image
OUTPUT_CHUNK = 8 * 1024
OUTPUT_INTERVAL = 0.01
ERROR = '{"error":"desktop preview unavailable"}'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def load_environment():
    override = os.environ.get('ART_TERM_PREVIEW_CONFIG', '')
    path = Path(override) if override else Path(__file__).resolve().parent / 'desktop.env'
    require(path.is_absolute(), 'preview config path must be absolute')
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and info.st_uid == os.getuid() and
            not stat.S_IMODE(info.st_mode) & 0o077,
            'preview config must be an owner-only regular file')
    raw = path.read_bytes()
    require(0 < len(raw) <= MAX_ENV_BYTES and b'\0' not in raw, 'preview config size invalid')
    values = {}
    for line in raw.decode('utf-8').split('\n'):
        line = line.rstrip('\r')
        if not line:
            continue
        match = ENV_LINE.fullmatch(line)
        require(match is not None, 'preview config is not a plain KEY=VALUE file')
        values[match.group(1)] = match.group(2)
    token_var = values.get('ART_TERM_DESKTOP_MCP_TOKEN_ENV') or 'HANDS_MCP_TOKEN'
    require(values.get('ART_TERM_DESKTOP_MCP_URL') and IDENT.fullmatch(token_var) and
            values.get(token_var), 'preview config missing MCP URL or token')
    # Mirror the previous EnvironmentFile semantics: the owner-managed file
    # defines the helper environment; only PATH is added around it.
    env = dict(values)
    env['PATH'] = os.environ.get('PATH', '/usr/local/bin:/usr/bin:/bin')
    return env


def run_helper(operation):
    env = load_environment()
    helper = Path(__file__).resolve().parent / 'mcp_preview.py'
    require(helper.is_file() and not helper.is_symlink(), 'preview helper missing')
    result = subprocess.run([sys.executable, str(helper), operation],
                            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                            stderr=subprocess.DEVNULL, env=env, timeout=HELPER_TIMEOUT)
    data = result.stdout
    require(0 < len(data) <= MAX_OUTPUT, 'helper output outside bound')
    line = data.split(b'\n', 1)[0]
    value = json.loads(line)
    require(isinstance(value, dict), 'helper reply is not a JSON object')
    return result.returncode, line.decode('utf-8')


def main():
    if (len(sys.argv) != 3 or sys.argv[1] not in ('status', 'screenshot') or
            not NONCE.fullmatch(sys.argv[2])):
        sys.stderr.write('usage: preview.py status|screenshot NONCE (32 lowercase hex)\n')
        return 2
    operation, nonce = sys.argv[1], sys.argv[2]
    out = sys.stdout
    # Interactive shells may leave prompt/bracketed-paste controls on this line.
    # Start a fresh line so the strict PTY frame parser sees an exact marker.
    out.write('\nARTTERM_PREVIEW_BEGIN:' + nonce + '\n')
    out.flush()
    try:
        code, line = run_helper(operation)
    except Exception:
        # Fixed non-secret failure; never echo env, paths or tracebacks.
        code, line = 1, ERROR
    # The official runtime drops old output if a subscriber's 256 KiB queue
    # fills. A browser PNG's base64 JSON can exceed that in a single write.
    # Pace this dedicated Shell's output so the unmodified runtime can drain
    # it; the App still rejects any gap instead of accepting a partial image.
    for offset in range(0, len(line), OUTPUT_CHUNK):
        if offset:
            time.sleep(OUTPUT_INTERVAL)
        out.write(line[offset:offset + OUTPUT_CHUNK])
        out.flush()
    out.write('\n')
    out.write('ARTTERM_PREVIEW_END:' + nonce + ':' + str(code) + '\n')
    out.flush()
    return code


if __name__ == '__main__':
    sys.exit(main())
