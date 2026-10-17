"""Real WebView2 native driver with pass-through IPC observations."""
import hashlib
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import time
import requests
from selenium import webdriver
from selenium.webdriver.common.by import By
from selenium.webdriver.common.action_chains import ActionChains
from selenium.webdriver.remote.client_config import ClientConfig
from selenium.webdriver.common.proxy import Proxy, ProxyType
from selenium.common.exceptions import StaleElementReferenceException

HIDDEN = getattr(subprocess, 'CREATE_NO_WINDOW', 0)


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def powershell(source):
    result = subprocess.run(['powershell.exe', '-NoProfile', '-NonInteractive', '-Command', source],
                            capture_output=True, timeout=20, creationflags=HIDDEN)
    require(result.returncode == 0, 'process ownership query failed')
    return json.loads(result.stdout.decode('utf-8-sig').strip() or '[]')


def apps():
    value = powershell("[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); ConvertTo-Json -Compress -InputObject @(Get-CimInstance Win32_Process -Filter \"Name='art-term-gui.exe'\" | ForEach-Object { @{pid=$_.ProcessId;parent=$_.ParentProcessId;path=$_.ExecutablePath;born=$_.CreationDate.ToUniversalTime().Ticks.ToString()} })")
    return value if isinstance(value, list) else [value]


def child_tree(root_pid):
    # Windows can retain an orphan's numeric ParentProcessId after PID reuse.
    # Every link must be newer than its actual parent, including the root.
    return powershell("[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); $all=@(Get-CimInstance Win32_Process); $treeRootId=" + str(int(root_pid)) + "; $treeRoot=@($all | Where-Object {$_.ProcessId -eq $treeRootId}); $bornById=@{}; if($treeRoot.Count -eq 1){$bornById[[uint32]$treeRootId]=$treeRoot[0].CreationDate}; $found=@(); do {$next=@($all | Where-Object {$bornById.ContainsKey([uint32]$_.ParentProcessId) -and -not $bornById.ContainsKey([uint32]$_.ProcessId) -and $_.CreationDate -ge $bornById[[uint32]$_.ParentProcessId]}); $found+=@($next); foreach($child in $next){$bornById[[uint32]$child.ProcessId]=$child.CreationDate}} while($next.Count); ConvertTo-Json -Compress -InputObject @($found | ForEach-Object {@{pid=$_.ProcessId;born=$_.CreationDate.ToUniversalTime().Ticks.ToString()}})")


OBSERVER = r"""
const internals=window.__TAURI_INTERNALS__;
if(!internals || window.__reg)throw Error('native observer unavailable');
window.__reg={sessions:{},acks:0,recoveries:0,shots:0,shotStarts:[]};
const r=window.__reg;
function record(cmd,a,v){
 if(cmd==='gui_open')r.sessions[v.id]={name:v.remoteSession,target:v.targetId,tail:'',decoder:new TextDecoder(),bytes:0};
 const s=r.sessions[a.sessionId];
 function collect(value){if(!value || typeof value!=='object')return; for(const [k,x] of Object.entries(value)){
   if(s && k==='bytes' && Array.isArray(x)){s.bytes+=x.length;s.tail=(s.tail+s.decoder.decode(new Uint8Array(x),{stream:true})).slice(-8192);}
   else if(x && typeof x==='object')collect(x);
 }}
 if(cmd==='gui_poll')collect(v);
 if(cmd==='gui_runtime_ack')r.acks++;
 if(cmd==='gui_runtime_recover')r.recoveries++;
 if(cmd==='gui_screenshot')r.shots++;
}
const original=internals.invoke.bind(internals);
const d=Object.getOwnPropertyDescriptor(internals,'invoke');
if(!d || d.writable || d.configurable){
 const wrapped=async(cmd,a={},options)=>{if(cmd==='gui_screenshot')r.shotStarts.push(performance.now());const v=await original(cmd,a,options);record(cmd,a,v);return v};
 if(d && !d.configurable)internals.invoke=wrapped;
 else Object.defineProperty(internals,'invoke',{value:wrapped,writable:true,configurable:true});
 r.mode='native-invoke';
}else{
 const originalFetch=window.fetch.bind(window);
 window.fetch=async(input,options)=>{
   const url=new URL(typeof input==='string'?input:input.url,location.href);
   if(url.hostname==='ipc.localhost' && decodeURIComponent(url.pathname.slice(1))==='gui_screenshot')r.shotStarts.push(performance.now());
   const response=await originalFetch(input,options);
   if(url.hostname==='ipc.localhost' && response.headers.get('Tauri-Response')==='ok'){
    const cmd=decodeURIComponent(url.pathname.slice(1));
    const args=options && typeof options.body==='string'?JSON.parse(options.body||'{}'):{};
    if(cmd.startsWith('gui_'))record(cmd,args,await response.clone().json());
   }
   return response;
 };r.mode='native-ipc-fetch';
}
return r.mode;
"""


class App:
    def __init__(self, exe, driver, run, fixture):
        self.exe, self.driver_exe, self.run, self.fixture = exe, driver, run, fixture
        self.driver = self.process = None
        self.owned = []
        self.cleanups = []
        self.launch_count = 0
        self.metadata = {}

    def wait(self, predicate, seconds=20):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            try:
                value = predicate()
            except StaleElementReferenceException:
                value = False
            if value:
                return value
            time.sleep(.12)
        raise RuntimeError('bounded wait expired')

    def launch(self, config):
        require(not apps(), 'existing GUI must be closed before isolated test')
        self.launch_count += 1
        env = dict(os.environ)
        env['WEBVIEW2_USER_DATA_FOLDER'] = str(self.run / ('webview-' + str(self.launch_count)))
        env['ART_TERM_OPENAB_FIXTURE_CA'] = str(self.fixture.ca)
        env.pop('ART_TERM_OPENAB_FIXTURE_DATA', None)
        env.pop('WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS', None)
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        url = f'http://127.0.0.1:{port}'
        self.log = (self.run / 'driver.log').open('ab')
        self.process = subprocess.Popen([str(self.driver_exe), '--port=' + str(port)], env=env,
                                        stdout=self.log, stderr=subprocess.STDOUT, creationflags=HIDDEN)
        http = requests.Session()
        http.trust_env = False
        def ready():
            require(self.process.poll() is None, 'driver exited before ready')
            try:
                return http.get(url + '/status', timeout=1).status_code == 200
            except requests.RequestException:
                return False
        self.wait(ready, 15)
        options = webdriver.EdgeOptions()
        options.use_webview = True
        options.binary_location = str(self.exe)
        options.add_argument('--config=' + str(config))
        client = ClientConfig(remote_server_addr=url, timeout=55,
                              proxy=Proxy({'proxyType': ProxyType.DIRECT['string']}))
        try:
            self.driver = webdriver.Remote(url, options=options, client_config=client)
        finally:
            self.owned = [p for p in apps() if p['parent'] == self.process.pid and Path(p['path']).resolve() == self.exe]
        require(len(self.owned) == 1, 'ambiguous test GUI ownership')
        self.driver.set_script_timeout(45)
        self.wait(lambda: self.find('[data-testid=desktop-workspace-layout]'))
        self.metadata['observer'] = self.driver.execute_script(OBSERVER)
        self.metadata['browserVersion'] = self.driver.capabilities.get('browserVersion')
        self.metadata['driverVersion'] = self.driver.capabilities.get('msedge', {}).get('msedgedriverVersion')
        # CDP is attached to this actual WebView; no browser-only replacement.
        self.driver.command_executor._commands['executeCdpCommand'] = ('POST', '/session/$sessionId/ms/cdp/execute')
        self.metadata['cdp'] = self.driver.execute('executeCdpCommand', {'cmd': 'Browser.getVersion', 'params': {}})['value']['product']
        require(self.metadata['browserVersion'], 'WebView version unavailable')
        actual_driver = re.search(r'\d+\.\d+\.\d+\.\d+', self.metadata['driverVersion'] or '')
        require(actual_driver and actual_driver[0].split('.')[:3] == self.metadata['browserVersion'].split('.')[:3], 'EdgeDriver/WebView2 version mismatch')
        return self

    def find(self, css):
        return self.driver.find_elements(By.CSS_SELECTOR, css)

    def node(self, testid):
        return self.wait(lambda: [e for e in self.find('[data-testid="' + testid + '"]') if e.is_displayed()])[-1]

    def click(self, text, css='button'):
        def find():
            for element in self.find(css):
                if element.text.strip() == text and element.is_displayed() and element.is_enabled():
                    return element
            return False
        self.wait(find).click()

    def fill(self, testid, text):
        element = self.node(testid)
        element.clear()
        element.send_keys(text)

    def invoke(self, cmd, args=None):
        reply = self.driver.execute_async_script("const [c,a,done]=arguments;window.__TAURI_INTERNALS__.invoke(c,a).then(v=>done({ok:true,value:v}),()=>done({ok:false}));", cmd, args or {})
        require(reply['ok'], 'native query rejected')
        return reply.get('value')

    def profiles(self):
        return self.invoke('gui_runtime_hosts')

    def manage(self):
        self.node('desktop-settings').click()
        self.node('settings-connections').click()
        self.node('runtime-host-form')

    def settings(self):
        self.node('desktop-settings').click()
        self.node('settings-screens').click()
        self.node('screen-settings')

    def main_page(self):
        for element in self.find('[data-testid=desktop-workspace]'):
            if element.is_displayed():
                element.click()
        self.wait(lambda: any(e.is_displayed() for e in self.find('.desktop-main-page')))

    def connect_page(self):
        self.main_page()

    def screens(self):
        return self.invoke('gui_instance_desktops')

    def terminal(self, text):
        textarea = self.wait(lambda: self.driver.execute_script("return [...document.querySelectorAll('.pane .xterm-helper-textarea')].find(e=>e.closest('.pane').getBoundingClientRect().width>0) || null;"))
        # xterm owns this real native input element. No direct gui_write injection.
        self.driver.execute_script('arguments[0].focus();', textarea)
        ActionChains(self.driver).send_keys(text).perform()

    def contains(self, name, text):
        return self.driver.execute_script("return Object.values(window.__reg.sessions).some(s=>s.name===arguments[0] && s.tail.includes(arguments[1]));", 'openab:' + name, text)

    def rendered(self, image=None):
        return self.driver.execute_script("const i=[...document.querySelectorAll('img.shot')].find(e=>e.getBoundingClientRect().width>0);return !!i && i.complete && i.naturalWidth===2 && i.naturalHeight===2 && (!arguments[0] || i.src===arguments[0]);", image)

    def shots(self):
        return self.driver.execute_script('return window.__reg.shots;')

    def picker(self, trigger, path):
        trigger()
        process = subprocess.Popen(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
                                    str(Path(__file__).with_name('native-picker.ps1')), '-AppProcessId', str(self.owned[0]['pid']),
                                    '-Action', 'choose', '-RunRoot', str(self.run), '-TestFile', str(path)],
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, creationflags=HIDDEN)
        try:
            out, _ = process.communicate(timeout=25)
            if process.returncode != 0:
                (self.run / 'native-picker-failure.log').write_bytes(out)
            require(process.returncode == 0 and json.loads(out.decode('utf-8-sig'))['invoked'], 'owned native picker failed')
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=5)

    def close(self):
        failure = None
        children = child_tree(self.process.pid) if self.process else []
        if self.driver:
            try:
                self.driver.quit()
            except Exception:
                failure = 'webdriver quit failed'
            self.driver = None
        if self.process:
            # If startup failed, reacquire only exact direct children of our driver.
            current = [p for p in apps() if p['parent'] == self.process.pid and Path(p['path']).resolve() == self.exe]
            for item in current:
                if item not in self.owned:
                    self.owned.append(item)
            for item in self.owned:
                pid, born = int(item['pid']), item['born']
                require(born.isdigit(), 'invalid process creation identity')
                powershell("$p=Get-CimInstance Win32_Process -Filter 'ProcessId=" + str(pid) + "'; if($p -and $p.CreationDate.ToUniversalTime().Ticks.ToString() -eq '" + born + "'){Stop-Process -Id " + str(pid) + " -ErrorAction Stop}; '[]'")
            for item in reversed(children):
                pid, born = int(item['pid']), item['born']
                require(born.isdigit(), 'invalid child process creation identity')
                powershell("$p=Get-CimInstance Win32_Process -Filter 'ProcessId=" + str(pid) + "'; if($p -and $p.CreationDate.ToUniversalTime().Ticks.ToString() -eq '" + born + "'){Stop-Process -Id " + str(pid) + " -ErrorAction Stop}; '[]'")
            if self.process.poll() is None:
                self.process.terminate()
                try:
                    self.process.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    self.process.kill()
                    self.process.wait(timeout=5)
            require(not any(p in apps() for p in self.owned), 'owned GUI cleanup failed')
            self.process = None
            self.log.close()
            self.cleanups.append({'ownedGuiStopped': True, 'driverStopped': True, 'ownedChildrenChecked': len(children), 'quitWarning': failure})
            self.owned = []
