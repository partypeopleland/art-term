"""CLI-neutral endpoint: filtered SDK env, live ancestor scope, no Host grant."""
import ctypes,json,os,shlex,socket,subprocess,sys,tempfile,time
from pathlib import Path
from shell_supervisor import Shell
from run import bridge,ROOT
from fixture import Fixture

with tempfile.TemporaryDirectory(prefix='art-generic-') as folder:
 home=Path(folder);(home/'.bashrc').write_text("export PS1='BROKER_PROMPT> '\n")
 one,two=Shell(home),Shell(home);a,b=Fixture('A'),Fixture('B')
 sockets=[]
 def start(shell,index):
  endpoint=home/f'client{index}.sock'
  shell.command('python3 '+shlex.quote(str(ROOT/'scripts/instance-mcp-regression/generic_client.py'))+' --socket '+shlex.quote(str(endpoint))+' & true')
  end=time.monotonic()+5
  while not endpoint.exists() and time.monotonic()<end:time.sleep(.05)
  assert endpoint.exists(),'generic client did not become ready'
  sockets.append(endpoint);return endpoint
 def tool(endpoint,name,args=None):
  with socket.socket(socket.AF_UNIX) as client:
   client.settimeout(25);client.connect(str(endpoint));bridge.send(client,{'method':'tools/call','params':{'name':name,'arguments':args or {}}});return bridge.receive(client)
 def target(endpoint):return json.loads(tool(endpoint,'current_target')['content'][0]['text'])['target']
 try:
  # Real non-dumpable same-UID parent above the broker; SDK proxies must not
  # depend on reading that unrelated process environment.
  assert ctypes.CDLL(None).prctl(4,0,0,0,0)==0
  first=start(one,1);second=start(two,2)
  assert ctypes.CDLL(None).prctl(4,1,0,0,0)==0
  boundary=one.command('python3 '+shlex.quote(str(ROOT/'scripts/instance-mcp-regression/ancestor_boundary.py'))+' '+str(os.getpid())).decode()
  assert 'ANCESTOR_BOUNDARY_PASS' in boundary

  assert tool(first,'current_target')['isError'] and tool(second,'current_target')['isError'] and not a.events and not b.events
  bridge.configure(one.binding,a.config());bridge.configure(two.binding,b.config())
  assert target(first)['id']=='desktop-A' and target(second)['id']=='desktop-B'
  ta=target(first)
  assert tool(first,'call_target_tool',{'name':'browser_title','arguments':{},'expected_target':ta})['_meta']['artTermTarget']==ta
  config=[]
  for shell in (one,two):
   output=shell.command('art-term-screen mcp-config').decode()
   line=next(line for line in output.splitlines() if line.startswith('{"command":'))
   data=json.loads(line);assert data['args'][-2:]==['stdio','--current-shell']
   assert shell.binding not in line and a.url not in line and b.url not in line and a.token not in line and b.token not in line
   config.append(data)
  assert config[0]==config[1],'generic config tied to a fixed Shell'
  denied=one.command('ART_TERM_SCREEN_BINDING='+shlex.quote(two.binding)+' python3 '+shlex.quote(str(ROOT/'scripts/instance_bridge.py'))+' stdio --current-shell </dev/null; echo AMBIGUOUS_STATUS=$?').decode()
  assert 'AMBIGUOUS_STATUS=1' in denied,'conflicting live Shell bindings were not refused'
  bridge.configure(one.binding,b.config());assert target(first)['id']=='desktop-B' and target(second)['id']=='desktop-B'
  assert tool(first,'call_target_tool',{'name':'browser_title','arguments':{},'expected_target':ta})['isError']
  bridge.configure(one.binding,{'op':'clear'});assert tool(first,'current_target')['isError'];assert target(second)['id']=='desktop-B'
  # An outside process must not attach to active unrelated Shells, even with a copied binding env.
  for binding in ('',two.binding):
   env=dict(os.environ,ART_TERM_SCREEN_BINDING=binding)
   p=subprocess.run([sys.executable,str(ROOT/'scripts/instance_bridge.py'),'stdio','--current-shell'],input=b'{"jsonrpc":"2.0","id":1,"method":"initialize"}\n',capture_output=True,env=env,timeout=5)
   assert p.returncode!=0 and not p.stdout,'unbound client selected unrelated live Shell'
 finally:
  ctypes.CDLL(None).prctl(4,1,0,0,0)
  one.close();two.close();a.close();b.close()
print('PASS: generic MCP filtered-env ancestor scope, twoShell isolation, zeroHTTP before grant, A/B/stale/clear, stable secret-free mcp-config, unrelated/unbound denial')
