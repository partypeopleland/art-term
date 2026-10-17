"""Native Rust configure → real official PTY → fixed AI → MCP A/B."""
import asyncio
import base64
import importlib.util
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import time
import uuid
from fixture import Fixture as McpFixture
from run import ROOT, bridge

sys.path.insert(0, str(ROOT/'scripts/windows-regression'))
# Load explicitly: both suites have a fixture.py with different responsibilities.
def load(name, path):
    s=importlib.util.spec_from_file_location(name,path); m=importlib.util.module_from_spec(s); s.loader.exec_module(m); return m
WireFixture=load('wire_fixture', ROOT/'scripts/windows-regression/fixture.py').Fixture
linux=load('linux_harness', ROOT/'scripts/linux-regression/run.py')
from aiohttp import ClientSession, web, WSMsgType


class TlsRuntimeGateway(WireFixture):
    def __init__(self, root, harness):
        self.harness, self.output, self.grants, self.replay = harness, [], {}, []
        self.live_ws = set()
        super().__init__(root)
        self.credential = harness.credential
    async def admin(self, request):
        target='http://127.0.0.1:'+str(self.harness.port)+request.raw_path
        # Before DELETE, reattach since=0 to inspect real Runtime replay output.
        name=request.match_info.get('name')
        if request.method == 'DELETE' and name in self.grants:
            grant=self.grants[name]
            async with ClientSession() as client:
                try:
                    async with client.ws_connect('http://127.0.0.1:'+str(self.harness.port)+'/pty/'+name+'?since=0', headers={'Authorization':'Bearer '+grant['token']}) as ws:
                        while True:
                            message=await asyncio.wait_for(ws.receive(), .2)
                            if message.type == WSMsgType.BINARY: self.replay.append(message.data)
                            elif message.type == WSMsgType.TEXT: self.replay.append(message.data.encode())
                            else: break
                except (asyncio.TimeoutError, Exception): pass
        async with ClientSession() as client:
            async with client.request(request.method,target,data=await request.read(),headers={'Authorization':request.headers.get('Authorization',''),'Content-Type':'application/json'}) as r:
                raw=await r.read()
                if request.method=='POST' and r.status==200:
                    obj=json.loads(raw)
                    if 'token' in obj: self.grants[obj['session']]=obj
                return web.Response(body=raw,status=r.status,content_type='application/json')
    async def pty(self, request):
        out=web.WebSocketResponse(); await out.prepare(request)
        self.live_ws.add(out)
        async with ClientSession() as client:
            async with client.ws_connect('http://127.0.0.1:'+str(self.harness.port)+request.raw_path,headers={'Authorization':request.headers.get('Authorization','')}) as remote:
                async def forward(source,target,capture=False):
                    async for message in source:
                        if message.type==WSMsgType.BINARY:
                            if capture: self.output.append(message.data)
                            await target.send_bytes(message.data)
                        elif message.type==WSMsgType.TEXT:
                            if capture: self.output.append(message.data.encode())
                            await target.send_str(message.data)
                        else: break
                tasks=[asyncio.create_task(forward(out,remote)),asyncio.create_task(forward(remote,out,True))]
                await asyncio.wait(tasks,return_when=asyncio.FIRST_COMPLETED)
                for t in tasks: t.cancel()
                await asyncio.gather(*tasks,return_exceptions=True)
        self.live_ws.discard(out)
        await out.close(); return out
    def close(self):
        async def stop():
            for ws in list(self.live_ws):
                if ws._req and ws._req.transport: ws._req.transport.abort()
            await self.runner.cleanup()
        try:
            self.call(stop())
        finally:
            self.loop.call_soon_threadsafe(self.loop.stop)
            self.thread.join(timeout=5)
            self.loop.close()


def main():
    root=ROOT/'target/linux-regression'/('instance-'+uuid.uuid4().hex);root.mkdir(parents=True,mode=0o700)
    h=linux.Harness(root,None)
    proxy=None; a=None;b=None;ws=None
    try:
        installed=h.home/'.local/share/art-term/tools';installed.mkdir(parents=True,mode=0o700)
        shutil.copyfile(ROOT/'scripts/instance_bridge.py',installed/'instance_bridge.py')
        proxyroot=root/'proxy';proxyroot.mkdir();proxy=TlsRuntimeGateway(proxyroot,h)
        a,b=McpFixture('A', protocol='2024-11-05'),McpFixture('B',sse=True)
        name,grant=h.create('work');ws=h.attach(name,grant)
        endpoint=root/'fixed-ai.sock'
        command=f'exec python3 {ROOT}/scripts/instance-mcp-regression/remote_client.py --binding {name} --socket {endpoint}\n'
        ws.send(2,command.encode());ws.collect(b'FIXED_AI_READY')
        def ai(method,params):
            with socket.socket(socket.AF_UNIX) as s:
                s.settimeout(25);s.connect(str(endpoint));bridge.send(s,{'method':method,'params':params});return bridge.receive(s)
        def tool(n,args=None): return ai('tools/call',{'name':n,'arguments':args or {}})
        def target(): return json.loads(tool('current_target')['content'][0]['text'])['target']
        def configure(payload):
            data={'origin':proxy.origin,'ca':str(proxy.ca),'credential':h.credential,'binding':name,'payload':payload}
            p=subprocess.run([str(ROOT/'target/debug/examples/configure_instance')],input=json.dumps(data).encode(),capture_output=True,timeout=65)
            assert p.returncode==0, p.stderr.decode()
            result=json.loads(p.stdout);assert result['result']['adapters']==1
            sessions=h.api('GET','/admin/sessions')[1]['sessions']
            assert all(s['name'] not in result['owned'] for s in sessions), 'management Shell leaked'
            return result
        result=configure(a.config());assert base64.b64decode(result['image'])==a.image
        t=target();r=tool('call_target_tool',{'name':'browser_navigate','arguments':{'url':'https://fixture.example/official-A'},'expected_target':t})
        assert not r.get('isError') and 'official-A' in r['content'][0]['text']
        # Renew changes Runtime generation but keeps adapter and its binding alive.
        status,renewed=h.api('POST','/admin/sessions/'+name+'/renew');assert status==200 and renewed['generation']>grant['generation']
        result=configure(b.config());assert base64.b64decode(result['image'])==b.image
        assert tool('call_target_tool',{'name':'browser_title','arguments':{},'expected_target':t})['isError']
        r=tool('call_target_tool',{'name':'browser_navigate','arguments':{'url':'https://fixture.example/official-B'},'expected_target':target()})
        assert '"desktop": "B"' in r['content'][0]['text']
        configure({'op':'clear'});assert tool('current_target')['isError']
        output=b''.join(proxy.output+proxy.replay)
        for secret in (a.token,b.token):
            assert secret.encode() not in output and base64.b64encode(secret.encode()) not in output, 'credential in PTY output'
            for proc in Path('/proc').glob('[0-9]*'):
                try:
                    cmd=(proc/'cmdline').read_bytes()
                    if b'instance_bridge.py' in cmd: assert secret.encode() not in cmd+(proc/'environ').read_bytes()
                except (PermissionError,ProcessLookupError,FileNotFoundError): pass
            for history in h.home.rglob('*history*'):
                if history.is_file(): assert secret.encode() not in history.read_bytes()
        assert proxy.replay, 'real Runtime replay path not observed'
        passed=True
    finally:
        if ws: ws.close()
        if proxy: proxy.close()
        if a: a.close()
        if b: b.close()
        h.close()

    assert passed
    print('PASS: official Runtime native configure, fixed AI tool A/B, direct native screenshots A/B, renew, clear, no output/history/argv/env token, management cleanup')

if __name__=='__main__': main()
