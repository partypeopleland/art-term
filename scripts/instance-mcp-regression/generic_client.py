"""External MCP SDK-shaped fixed client; deliberately drops custom environment."""
import argparse,json,os,select,socketserver,subprocess,sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
from run import bridge,ROOT
parser=argparse.ArgumentParser();parser.add_argument('--socket',required=True);args=parser.parse_args()
# Preserve only ordinary essentials, as an MCP SDK may do.
env={k:os.environ[k] for k in ('HOME','PATH','LANG') if k in os.environ}
child=subprocess.Popen([sys.executable,str(ROOT/'scripts/instance_bridge.py'),'stdio','--current-shell'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=env)
n=0

def call(method,params):
 global n
 n+=1;child.stdin.write((json.dumps({'jsonrpc':'2.0','id':n,'method':method,'params':params})+'\n').encode());child.stdin.flush()
 assert select.select([child.stdout],[],[],5)[0],'generic proxy timeout'
 raw=child.stdout.readline();assert raw,child.stderr.read().decode()
 reply=json.loads(raw);assert reply['id']==n;return reply['result']
call('initialize',{})
class Handler(socketserver.BaseRequestHandler):
 def handle(self):
  body=bridge.receive(self.request);bridge.send(self.request,call(body['method'],body.get('params',{})))
server=socketserver.UnixStreamServer(args.socket,Handler)
print('GENERIC_CLIENT_READY',flush=True)
try:server.serve_forever()
finally:child.stdin.close();child.wait(timeout=5);server.server_close()
