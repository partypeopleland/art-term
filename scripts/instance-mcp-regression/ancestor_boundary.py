"""Run inside real Shell; irrelevant ancestor permission failure must not be read."""
import os,sys
from run import bridge
outside=int(sys.argv[1]);original=bridge.ancestor_binding

def guarded(pid):
 if pid==outside:raise PermissionError('fixture inaccessible ancestor environment')
 return original(pid)
bridge.ancestor_binding=guarded
binding,path,peer=bridge.current_shell()
assert peer[0] != outside
print('ANCESTOR_BOUNDARY_PASS',flush=True)
