"""Only owned throwaway HOME: narrow generic installation, backup and refusal."""
import os, subprocess, tempfile
from pathlib import Path
from run import ROOT

with tempfile.TemporaryDirectory(prefix='art-shell-install-') as folder:
    home=Path(folder);image=home/'image-tmux';image.mkdir(mode=0o700)
    original=image/'tmux-shell.py';original.write_text('image owned wrapper\n');original.chmod(0o700)
    tmux=home/'.local/share/art-term/openab-pty/tmux-v2';tmux.parent.mkdir(parents=True,mode=0o700);tmux.symlink_to(image,target_is_directory=True)
    tools=home/'.local/share/art-term/tools';tools.mkdir(mode=0o700)
    wrapper=tools/'tmux-shell.py';wrapper.write_text('old overlay wrapper\n');wrapper.chmod(0o700)
    config=home/'.codex/config.toml';config.parent.mkdir();config.write_text('untouched fixture settings\n')
    env=dict(os.environ,HOME=str(home))
    command=['bash',str(ROOT/'scripts/install-instance-tools.sh'),'--with-tmux-wrapper']
    first=subprocess.run(command,env=env,capture_output=True,timeout=10);assert first.returncode==0,first.stderr
    backups=list(tools.glob('*.pre-shell-screen-*'));assert len(backups)==1 and backups[0].read_bytes()==b'old overlay wrapper\n'
    assert backups[0].stat().st_mode & 0o777==0o600
    for name in ('instance_bridge.py','instance-shell.py','instance-shell.bash'):
        path=home/'.local/share/art-term/tools'/name
        assert path.read_bytes()==(ROOT/'scripts'/name).read_bytes()
        assert path.stat().st_mode & 0o777==0o700
    assert wrapper.read_bytes()==(ROOT/'scripts/openab-runtime/tmux-v2/tmux-shell.py').read_bytes()
    assert subprocess.run(command,env=env,capture_output=True,timeout=10).returncode==0
    assert list(tools.glob('*.pre-shell-screen-*'))==backups
    assert tmux.is_symlink() and tmux.resolve()==image
    assert original.read_text()=='image owned wrapper\n'
    assert not list(image.glob('*.pre-shell-screen-*'))
    assert config.read_text()=='untouched fixture settings\n'
    wrapper.unlink();wrapper.symlink_to(config)
    assert subprocess.run(command,env=env,capture_output=True,timeout=10).returncode!=0
    assert config.read_text()=='untouched fixture settings\n'
print('PASS: generic3+persistentoptionaloverlay exact files/modes, image symlink untouched, private original backup, idempotence, symlink refusal, no globalCLI settings changes')
