"""Actual SGID inheritance must produce private directories; unsafe modes denied."""
import os,stat,tempfile
from pathlib import Path
from run import bridge
with tempfile.TemporaryDirectory(prefix='art-sgid-') as folder:
 parent=Path(folder);parent.chmod(0o2700)
 child=parent/'broker';child.mkdir(mode=0o700)
 assert stat.S_IMODE(child.stat().st_mode)==0o2700,'fixture filesystem did not inherit SGID'
 bridge.private_directory(child)
 assert stat.S_IMODE(child.stat().st_mode)==0o700
 another=parent/'new';bridge.private_directory(another)
 assert stat.S_IMODE(another.stat().st_mode)==0o700
 child.chmod(0o2770)
 try:bridge.private_directory(child);raise AssertionError('unsafe access permissions accepted')
 except ValueError:pass
 assert stat.S_IMODE(child.stat().st_mode)==0o2770
 link=parent/'symlink';link.symlink_to(another,target_is_directory=True)
 try:bridge.private_directory(link);raise AssertionError('symlink accepted')
 except OSError:pass
print('PASS: real SGID-parent new/existing2700 repaired to700,2770 denied unchanged, symlink denied')
