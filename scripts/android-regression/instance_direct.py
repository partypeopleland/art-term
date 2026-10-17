"""Direct Screen UI on an owned emulator: native IPC, real HTTP MCP A/B, no AI."""
import argparse,base64,hashlib,importlib.util,json,time,tomllib,uuid
from pathlib import Path
from device import ROOT,PACKAGE,require,wait
from linux_emulator import LinuxEmulator
from run import Fixture as PtyFixture
from ui_refresh import Refresh
spec=importlib.util.spec_from_file_location('direct_mcp_fixture',ROOT/'scripts/instance-mcp-regression/fixture.py')
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)

class Screens:
    def __init__(self,suite,a,b):self.suite,self.a,self.b=suite,a,b;self.ids=[]
    def save(self,fixture):
        app=self.suite.app
        app.set('screen-name','Screen '+fixture.name);app.set('screen-url',fixture.url);app.set('screen-token',fixture.token)
        app.click_text('儲存 Screen','[data-testid=screen-form] button')
        wait(lambda: not app.evaluate('document.querySelector("[data-testid=screen-name]").disabled'))
        wait(lambda: app.evaluate('document.querySelectorAll(".saved-screens li").length')==len(self.ids)+1)
        require(app.evaluate('document.querySelector("[data-testid=screen-token]").value')=='','Screen token input retained')
        raw=self.suite.device.store();doc=tomllib.loads(raw.decode('utf-8-sig'))
        require(doc['schema_version']==6 and len(doc['connections'])==1,'Screen write lost Host or schema6')
        require(self.a.token.encode() not in raw and self.b.token.encode() not in raw,'plaintext Screen token in store')
        desktop=next(item for item in doc['desktops'] if item['name']=='Screen '+fixture.name)
        require(desktop.get('token_ciphertext'),'encrypted token missing')
        self.ids.append(desktop['id'])
    def catalog(self):
        s=self.suite;s.settings();wait(lambda:s.app.has('[data-testid=screen-form]'))
        self.save(self.a);self.save(self.b)
        require(not self.a.events and not self.b.events,'saving/listing Screens contacted MCP')
        s.app.close();s.device.stop();s.launch();s.page('Main')
        wait(lambda:s.app.has('[data-testid="screen-'+self.ids[1]+'"]'))
        require(s.app.profiles()==[s.profile],'Screen restart changed Host metadata')
        require(s.app.text('[data-testid=cat-screens] .section-count')=='02','Screen count absent')
        return {'keystoreScreensRestart':True,'hostPreserved':True,'metadataOnlyList':True,'encryptedTokenNoReadback':True}
    def choose(self,index,fixture):
        s=self.suite;a=s.app
        a.tap('desktop-toggle');wait(lambda:a.has('[data-testid=screen-picker]'))
        wait(lambda:a.evaluate('window.__scrollTest.nativeSnapshots.at(-1)?.visible===false'))
        a.tap('choose-screen-'+self.ids[index])
        expected='data:image/png;base64,'+base64.b64encode(fixture.image).decode()
        wait(lambda:a.evaluate('document.querySelector(".preview img")?.src')==expected,45)
        require(not a.has('[data-testid=screen-picker]'),'Screen picker stayed open')
        require(any(e['method']=='tools/call' and e['params'].get('name')=='screenshot' for e in fixture.events),'direct MCP screenshot missing')
        require(not any(e['method']=='tools/list' for e in fixture.events),'App preview unexpectedly explored AI tools')
    def preview(self):
        s=self.suite;a=s.app
        self.choose(0,self.a);require(not a.has('.preview .meta'),'preview contains removed metadata rows')
        s.device.screenshot(s.output/'direct-screen-a.png')
        self.choose(1,self.b);s.device.screenshot(s.output/'direct-screen-b.png')
        before=sum(e['method']=='POST' and e.get('action') is None for e in s.fixture.snapshot()['events'])
        calls=len(self.b.events)
        a.click('.preview-head .mini')
        wait(lambda:len(self.b.events)>calls)
        wait(lambda:not self.b.sessions)
        after=sum(e['method']=='POST' and e.get('action') is None for e in s.fixture.snapshot()['events'])
        require(after==before,'direct refresh created a Runtime screenshot Shell')
        inputs=''.join(item['input'] for item in s.fixture.snapshot()['sessions'].values())
        require(base64.b64encode(self.a.image).decode() not in inputs and base64.b64encode(self.b.image).decode() not in inputs,'MCP image relayed through PTY')
        a.tap('desktop-toggle');wait(lambda:a.has('[data-testid=screen-picker]'));a.tap('clear-screen')
        wait(lambda:not a.has('.preview img'))
        wait(lambda:'預覽已停止' in a.text('.preview'))
        wait(lambda:not self.a.sessions and not self.b.sessions)
        return {'nativeDirectScreenshotA':True,'nativeDirectScreenshotBSse':True,'previewWithoutAdapter':True,'noRuntimeScreenshotShell':True,'noImageInPty':True,'clearStopsPreview':True}

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--apk',required=True,type=Path);args=parser.parse_args()
    output=ROOT/'target/android-regression'/('instance-direct-'+uuid.uuid4().hex)
    result={'passed':False,'realApk':True,'mockedIpc':False,'cases':{},'limits':['x86_64 debug owned emulator; physical ARM64 phone pending','MCP services fixed fixtures; no real AI CLI/model','PTY wire fixture does not execute remote commands; official-Runtime adapter tested separately']}
    device=pty=suite=a=b=None;stage='boot'
    print('OUTPUT '+str(output),flush=True)
    try:
        device=LinuxEmulator(Path('/home/art/Android/Sdk'),output);device.start_emulator();device.install(args.apk.resolve())
        installed=device.text('shell','pm','path',PACKAGE).removeprefix('package:')
        result['apkSha256']=hashlib.sha256(args.apk.read_bytes()).hexdigest();require(device.text('shell','sha256sum',installed).split()[0]==result['apkSha256'],'installed APK mismatch')
        pty=PtyFixture(output);device.reverse(int(pty.origin.rsplit(':',1)[1]));device.private_write('no_backup/openab-operator-v1/openab-fixture-ca.der',pty.ca.read_bytes())
        a=module.Fixture('A',protocol='2024-11-05');b=module.Fixture('B',sse=True)
        for service in (a,b):device.reverse(service.server.server_port)
        suite=Refresh(device,pty,output);suite.launch();screens=Screens(suite,a,b)
        cases=[('empty-connect-swipe',suite.empty_swipe),('homepage',suite.about),('host-keystore',suite.settings_form),('screens-keystore',screens.catalog),('explicit-create',suite.create),('native-vertical',suite.vertical_drag),('swipe',suite.gestures),('keys',suite.keys),('theme',suite.theme),('direct-screens',screens.preview),('voice-lightbox',suite.lightbox),('background-saf',suite.secrets_and_saf),('finished-shell',suite.finished_shell)]
        for stage,action in cases:
            suite.wait_stage=stage;checks=action();result['cases'][stage]={'passed':True,'checks':checks};device.screenshot(output/(stage+'.png'));print('PASS '+stage,flush=True)
        result['passed']=True
    except Exception as error:
        result['failure']={'stage':stage,'errorType':type(error).__name__,'action':suite.app.action if suite and suite.app else None,'wait':suite.wait_stage if suite else None}
        if suite and suite.app:
            try:
                html=suite.app.evaluate('document.documentElement.outerHTML')
                for secret in (a.token if a else '',b.token if b else '',pty.credential if pty else ''):
                    if secret:html=html.replace(secret,'[redacted]')
                (output/'failure-dom.html').write_text(html);device.screenshot(output/'failure.png')
            except Exception:pass
        print('FAIL '+stage+' '+type(error).__name__,flush=True)
    finally:
        for name,cleanup in [('cdp',suite.app.close if suite and suite.app else None),('mcpA',a.close if a else None),('mcpB',b.close if b else None),('pty',pty.close if pty else None),('device',device.cleanup if device else None),('emulator',device.shutdown if device else None)]:
            if cleanup:
                try:cleanup();result.setdefault('cleanup',{})[name]=True
                except Exception:result.setdefault('cleanup',{})[name]=False;result['passed']=False
        output.mkdir(parents=True,exist_ok=True)
        (output/'result.json').write_text(json.dumps(result,indent=2)+'\n')
    return 0 if result['passed'] else 1
if __name__=='__main__':raise SystemExit(main())
