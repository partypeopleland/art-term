#!/usr/bin/env bash
set -euo pipefail
source_dir=$(cd "$(dirname "$0")" && pwd)
with_tmux=0
if [[ ${1:-} == --with-tmux-wrapper && $# == 1 ]]; then with_tmux=1
elif [[ $# != 0 ]]; then echo 'usage: install-instance-tools.sh [--with-tmux-wrapper]' >&2; exit 2; fi
# Generic files only: no endpoint, grant, CLI settings or service restarts.
python3 - "$source_dir" "$with_tmux" <<'PY'
import ast, hashlib, os
from pathlib import Path
import stat, subprocess, sys, tempfile, time, uuid
source = Path(sys.argv[1]); home = Path.home()
destination = home/'.local/share/art-term/tools'
files = [(source/name, destination/name) for name in ('instance_bridge.py','instance-shell.py','instance-shell.bash')]
if sys.argv[2] == '1':
    files.append((source/'openab-runtime/tmux-v2/tmux-shell.py',destination/'tmux-shell.py'))

def owned_directory(path, create=False):
    for parent in reversed([path,*path.parents]):
        if parent.is_symlink(): raise ValueError('symlink directory')
    if create: path.mkdir(parents=True,exist_ok=True,mode=0o700)
    info=path.stat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid(): raise ValueError('directory owner')
    if info.st_mode & 0o022: raise ValueError('writable directory')

owned_directory(destination,True)
os.chmod(destination,0o700)
for src,dst in files:
    owned_directory(dst.parent)
    if src.suffix=='.py': ast.parse(src.read_text())
    else: subprocess.run(['bash','-n',str(src)],check=True)
    if dst.exists() or dst.is_symlink():
        info=dst.lstat()
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid(): raise ValueError('file owner/type')
        if info.st_mode & 0o022: raise ValueError('writable file')

stamp=time.strftime('%Y%m%dT%H%M%SZ',time.gmtime())+'-'+uuid.uuid4().hex[:8]
for src,dst in files:
    data=src.read_bytes()
    if dst.exists() and dst.read_bytes()==data:
        os.chmod(dst,0o700)
    else:
        if dst.exists():
            backup=dst.with_name(dst.name+'.pre-shell-screen-'+stamp)
            fd=os.open(backup,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
            with os.fdopen(fd,'wb') as stream: stream.write(dst.read_bytes());stream.flush();os.fsync(stream.fileno())
            print('backup',backup)
        fd,temp=tempfile.mkstemp(prefix='.'+dst.name+'.',dir=dst.parent)
        try:
            with os.fdopen(fd,'wb') as stream: stream.write(data);stream.flush();os.fsync(stream.fileno())
            os.chmod(temp,0o700);os.replace(temp,dst)
        finally:
            if os.path.exists(temp):os.unlink(temp)
    assert dst.read_bytes()==data
    print(hashlib.sha256(data).hexdigest(),dst)
PY
