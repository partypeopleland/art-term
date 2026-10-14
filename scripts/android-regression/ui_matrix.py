"""Real APK UI matrix on an owned Linux AVD and the existing TLS/PT Y fixture.

No product IPC is replaced. Each case records its own result; dependencies
are explicitly blocked after a failed setup, independent groups still run.
"""
import argparse
from datetime import datetime, timezone
import hashlib
from PIL import Image, ImageChops
import io
import json
from pathlib import Path
import re
import subprocess
import _thread
import threading
import time
import uuid
from cdp import App
from device import ROOT, STORE, require, wait
from linux_emulator import LinuxEmulator
from reconnect_scroll import OBSERVE, ReconnectTest
from run import Suite, Fixture


class CaseFailure(RuntimeError):
    def __init__(self, message, evidence):
        super().__init__(message)
        self.evidence = evidence


class Matrix(Suite):
    def __init__(self, device, fixture, apk, run, case_ids=None):
        super().__init__(device, fixture, apk, run)
        inventory = json.loads((Path(__file__).with_name('ui-cases.json')).read_text())['cases']
        if case_ids is not None: inventory = [c for c in inventory if c['id'] in case_ids]
        self.results = {c['id']:{'title':c['title'],'status':'NOT_RUN', 'reason':'not reached or prerequisite failed'} for c in inventory}
        self.current = None
        self.save()

    def launch(self):
        a = super().launch()
        a.evaluate(OBSERVE)
        return a

    def save(self):
        (self.run/'cases.json').write_text(json.dumps(self.results, ensure_ascii=False, indent=2)+'\n')

    def case(self, identifier, title, action):
        require(identifier in self.results, 'Case missing from ui-cases.json: '+identifier)
        self.current = identifier
        started = time.monotonic()
        try:
            evidence = action()
            row = {'status':'PASS', 'title':title, 'evidence':evidence}
            self.device.screenshot(self.run/(identifier+'-pass.png'))
        except Exception as error:
            message = str(error).replace(self.fixture.credential, '[redacted]').replace(self.fixture.origin, '[fixture]')
            row = {'status':'FAIL', 'title':title, 'errorType':type(error).__name__, 'error':message[:350]}
            row['action'] = getattr(self.app, 'action', '')
            if hasattr(error,'evidence'): row['evidence'] = error.evidence
            try:
                self.device.screenshot(self.run/(identifier+'-failure.png'))
                row['touch'] = getattr(self.app, 'touch_diagnostic', {})
                row['screen'] = self.app.evaluate('({title:document.title,bodyClass:document.body.className})')
            except Exception:
                row['captureUnavailable'] = True
        row['seconds'] = round(time.monotonic()-started, 2)
        self.results[identifier] = row
        self.save()
        print(identifier+' '+row['status']+' '+title, flush=True)
        return row['status'] == 'PASS'

    def blocked(self, identifier, title, reason):
        self.results[identifier] = {'title':title,'status':'BLOCKED','reason':reason}
        self.save()

    def reset(self):
        if self.app: self.app.close()
        self.device.stop(); self.device.clear()
        self.device.private_write('no_backup/openab-operator-v1/openab-fixture-ca.der', self.fixture.ca.read_bytes())
        self.launch()

    def close_manage(self):
        self.app.click_text('關閉', '.runtime-hosts > header button')
        wait(lambda: not self.app.has('[data-testid=runtime-create]'))

    def select(self, testid, key):
        # A real keyboard interaction with the Android select control.
        self.app.evaluate(f'document.querySelector("[data-testid={testid}]").focus()')
        self.app.key(key, key); self.app.key('Enter', 'Enter')

    def home_empty(self):
        a = self.app
        require(a.profiles() == [], 'initial profiles not empty')
        require(a.has('[data-testid=runtime-host-settings]'), 'new connection entry absent')
        require('設定中沒有可用的連線' in a.text('.tree'), 'empty home guidance absent')
        a.tap('refresh-catalog')
        wait(lambda: not a.evaluate('document.querySelector("[data-testid=refresh-catalog]").disabled'))
        a.manage()
        require(a.evaluate('document.querySelector("[data-testid=runtime-export]").disabled'), 'empty export allowed')
        self.close_manage()
        return {'emptyAndRefresh':True, 'emptyExportDisabled':True}

    def cancel_host(self):
        a = self.app
        a.tap('runtime-host-settings'); a.set('name', 'DISCARD_ME')
        a.tap('runtime-form-cancel')
        require(a.profiles() == [], 'cancel saved profile')
        self.close_manage()

    def invalid_hosts(self):
        a = self.app
        a.manage(); a.tap('runtime-create')
        for name, url, credential in [(' ',self.fixture.origin,self.fixture.credential),
                                      ('test','http://127.0.0.1',self.fixture.credential),
                                      ('test',self.fixture.origin+'/path',self.fixture.credential),
                                      ('test',self.fixture.origin,''),
                                      ('繁'*50,self.fixture.origin,self.fixture.credential)]:
            a.set('name',name); a.set('base-url',url); a.set('credential',credential)
            a.click_text('儲存','[data-testid=runtime-host-form] button')
            wait(lambda: bool(a.text('#runtime-host-error')))
            require(a.profiles() == [], 'invalid input created profile')
        a.tap('runtime-form-cancel'); self.close_manage()
        return {'equivalenceClasses':['blank name','HTTP','nonroot URL','missing credential','UTF8 name over128bytes']}

    def delete_cancel(self):
        a = self.app; before = a.profiles()
        a.manage(); a.click_text('刪除','[data-host-id] button')
        a.click_text('取消','.runtime-hosts button')
        require(a.profiles() == before, 'cancel deletion changed profiles')
        self.close_manage()

    def auth_error(self):
        a = self.app; a.manage()
        a.click_text('編輯','[data-host-id] button'); a.set('credential','0'*64)
        a.click_text('儲存','[data-testid=runtime-host-form] button')
        wait(lambda: a.has('[data-testid=runtime-host-list]'))
        a.click_text('測試','[data-host-id] button')
        wait(lambda: bool(a.text('#runtime-host-error')))
        require('已通過管理認證' not in a.text('[data-host-id] [role=status]'), 'bad auth reported success')
        a.click_text('編輯','[data-host-id] button'); a.set('credential',self.fixture.credential)
        a.click_text('儲存','[data-testid=runtime-host-form] button'); self.test_host()
        self.close_manage()

    def filters(self):
        a = self.app; profile = a.profiles()[0]
        self.select('runtime-category-filter','ArrowDown')
        require(a.has('[data-testid="runtime-'+profile['id']+'"]'), 'Host filter hid Host')
        self.select('runtime-category-filter','ArrowDown')
        require(not a.has('[data-testid="runtime-'+profile['id']+'"]'), 'Agent filter showed Host')
        self.select('runtime-category-filter','Home')
        require(a.has('[data-testid="runtime-'+profile['id']+'"]'), 'All filter did not restore Host')

    def shell_validation(self):
        a = self.app
        a.tap('runtime-shell-'+a.profiles()[0]['id'])
        for name in ['', 'UPPER', 'has space', 'bad_name']:
            a.fill('.runtime-sessions form input',name)
            a.click_text('建立並連線','.runtime-sessions form button')
            wait(lambda: bool(a.text('#shell-error')))
        a.fill('.runtime-sessions form input','a'*32)
        a.command('Input.insertText',{'text':'b'})
        require(a.evaluate('document.querySelector(".runtime-sessions form input").value.length')==32,'Shell maxlength boundary failed')
        a.tap('sessions-return')

    def compose_cancel(self):
        a = self.app; before = self.fixture.snapshot()['sessions']['work-a']['input']
        a.tap('k-composer'); a.set('composer-text','MUST_NOT_SEND'); a.tap('composer-cancel')
        require(not a.has('[data-testid=composer-text]'), 'composer still open')
        require(self.fixture.snapshot()['sessions']['work-a']['input']==before, 'cancel sent input')

    def compose_plain(self):
        a = self.app; before = self.fixture.snapshot()['sessions']['work-a']['input']
        a.tap('k-composer'); a.set('composer-text','PLAIN_繁體'); a.tap('composer-send')
        wait(lambda: self.fixture.snapshot()['sessions']['work-a']['input']==before+'PLAIN_繁體')
        wait(lambda: self.contains('work-a','PLAIN_繁體'))

    def compose_enter(self):
        before = self.fixture.snapshot()['sessions']['work-a']['input']
        self.compose('ENTER_繁體')
        wait(lambda: self.fixture.snapshot()['sessions']['work-a']['input']==before+'ENTER_繁體\r')
        wait(lambda: 'ENTER_繁體' in self.app.evaluate(r'window.__scrollTest.records.map(r=>r.text).join("\n")'))
        require(any('ENTER_繁體' in n.get('text','') for n in self.device.ui().iter('node')), 'Unicode absent from native visible text tree')

    def key_bytes(self, testid, expected):
        before = self.fixture.snapshot()['sessions']['work-a']['input']
        self.app.tap(testid)
        wait(lambda: self.fixture.snapshot()['sessions']['work-a']['input']==before+expected)

    def ctrl(self):
        before = self.fixture.snapshot()['sessions']['work-a']['input']
        self.app.tap('k-ctrl')
        self.app.command('Input.insertText', {'text':'c'})
        wait(lambda: self.fixture.snapshot()['sessions']['work-a']['input']==before+'\x03')
        self.app.tap('k-ctrl')
        require(not self.app.evaluate('document.querySelector("[data-testid=k-ctrl]").classList.contains("on")'), 'Ctrl latch not cleared')

    def cli(self):
        self.app.tap('launch-coding-cli')
        wait(lambda: "'fixture-cli' '--literal' 'a b'\r" in self.fixture.snapshot()['sessions']['work-a']['input'])
        return {'wireCommand':"'fixture-cli' '--literal' 'a b'\\r", 'fixtureDoesNotExecuteCLI':True}

    def voice_swap(self):
        a = self.app
        wait(lambda:a.has('[data-testid=voice-settings-toggle]'))
        require(a.has('[data-testid=voice-preview]'), 'voice preview missing')
        a.tap('voice-settings-toggle')
        require(a.has('[data-testid=voice-settings]') and not a.has('[data-testid=voice-preview]'), 'settings did not replace preview')
        nodes = self.device.ui()
        require(any(n.get('class')=='android.widget.ScrollView' for n in nodes.iter('node')), 'settings hid native reader')
        a.click('input[name=voice-mode][value=hold]')
        require(a.evaluate('document.querySelector("input[value=hold]").checked'), 'hold mode not selected')
        a.click('.language-switch input')
        a.tap('voice-settings-toggle')
        require(a.has('[data-testid=voice-preview]') and not a.has('[data-testid=voice-settings]'), 'second toggle did not restore preview')
        a.tap('voice-settings-toggle')
        require(a.evaluate('document.querySelector("input[value=hold]").checked&&!document.querySelector(".language-switch input").checked'), 'settings lost during swap')
        a.click('input[name=voice-mode][value=tap]')
        require(a.evaluate('document.querySelector("input[value=tap]").checked'), 'tap mode not selected')
        a.tap('voice-settings-toggle')

    def voice_restart(self):
        self.restart()
        a = self.app
        a.tap('runtime-shell-'+a.profiles()[0]['id'])
        index = wait(lambda:a.evaluate('[...document.querySelectorAll(".runtime-sessions li")].findIndex(e=>e.querySelector("strong").textContent.trim()==="work-a")+1'))
        a.click(f'.runtime-sessions li:nth-child({index}) button')
        wait(lambda:a.has('[data-testid=voice-settings-toggle]'))
        a.tap('voice-settings-toggle')
        require(a.evaluate('document.querySelector("input[value=tap]").checked&&!document.querySelector(".language-switch input").checked'), 'voice settings lost on restart')
        a.tap('voice-settings-toggle')

    def voice_unavailable(self):
        a = self.app
        if a.has('[data-testid=voice-settings]'): a.tap('voice-settings-toggle')
        require(a.evaluate('document.querySelector("[data-testid=voice-mic]").disabled'), 'unexpected recognizer availability; use speech-enabled suite')
        require('此裝置不支援語音輸入' in a.text('[data-testid=voice-status]'), 'unavailable recognizer has no specific feedback')
        return {'recognitionTested':False,'disabledMicWithNotice':True, 'notice':a.text('[data-testid=voice-status]')}

    def modal_reader(self):
        a = self.app
        a.tap('k-composer')
        require(not any(n.get('class')=='android.widget.ScrollView' for n in self.device.ui().iter('node')), 'native reader obscures composer')
        a.tap('composer-cancel')
        a.tap('runtime-hosts-session-open')
        require(not any(n.get('class')=='android.widget.ScrollView' for n in self.device.ui().iter('node')), 'native history obscures management')
        self.close_manage()

    def shell_listing(self):
        a = self.app
        a.tap('to-list'); a.tap('runtime-shell-'+a.profiles()[0]['id'])
        a.click_text('重新整理 Shell','.runtime-sessions button')
        wait(lambda: a.has('.runtime-sessions li'))
        require('work-a' in a.text('.runtime-sessions') and '使用中' in a.text('.runtime-sessions') and '接管會中斷' in a.text('.runtime-sessions'), 'attached Shell state/warning absent')
        # Duplicate create must fail visibly without losing existing session.
        before = self.fixture.snapshot()['sessions']['work-a']['generation']
        a.fill('.runtime-sessions form input','work-a'); a.click_text('建立並連線','.runtime-sessions form button')
        wait(lambda: bool(a.text('#shell-error')))
        require(self.fixture.snapshot()['sessions']['work-a']['generation']==before,'duplicate creation replaced Shell')
        a.tap('sessions-return')
        # Return from homepage Shell chooser goes to homepage.
        wait(lambda: not a.has('.runtime-sessions'))
        a.click('[data-testid="runtime-'+a.profiles()[0]['id']+'"] .t-main')
        wait(lambda: a.has('[data-testid=link-status]') and a.evaluate('document.querySelector("[data-testid=to-list]").getBoundingClientRect().width>0'))
        self.tab('work-a')

    def append_stays(self):
        t = self.history_test()
        before, box = t.capture_crop('append-before')
        self.fixture.call(self.fixture.output(self.fixture.sessions['work-a'],b'NEW_OUTPUT_WHILE_READING\r\n'))
        wait(lambda:'NEW_OUTPUT_WHILE_READING' in t.observed_history())
        time.sleep(.4)
        after, newbox = t.capture_crop('append-after')
        require(newbox==box,'append changed reader bounds')
        changed = sum(max(p)>20 for p in ImageChops.difference(before,after).getdata())/(before.width*before.height)
        require(changed<.005,'new output pulled reader away from historical position')
        return {'changedPixelRatio':changed}

    def preview_visibility(self):
        a = self.app
        a.tap('desktop-toggle'); wait(self.rendered,25)
        a.tap('preview-visibility')
        require(not a.has('img.shot') and '已隱藏' in a.text('.preview'),'hide preview did not hide image')
        require(a.evaluate('document.querySelector(".preview-head button").disabled'),'hidden manual capture enabled')
        a.tap('preview-visibility'); wait(self.rendered,25)
        require(a.evaluate('document.querySelector("[data-testid=desktop-toggle]").getAttribute("aria-pressed")==="true"'),'show lost desktop enablement')
        a.tap('desktop-toggle')
        wait(lambda:not a.has('.preview'))

    def preview(self):
        a, f = self.app, self.fixture
        self.tab('work-a')
        started = time.monotonic()
        a.tap('desktop-toggle')
        try:
            wait(lambda:self.shots()>=1 and self.rendered(),25)
            first = self.shots(); elapsed = time.monotonic()-started
            require(elapsed<20,'first preview waited for periodic tick')
            wait(lambda:self.shots()>=first+2 and self.rendered(),75)
            starts = a.evaluate('window.__reg.shotStarts')
            interval = (starts[-1]-starts[-2])/1000
            require(24<=interval<=38,'production preview interval mismatch')
            # A distinct valid external image proves manual refresh replaces
            # and decodes the new response, rather than counting an old image.
            previous_src = a.evaluate('document.querySelector("img.shot").src')
            image = Image.open(io.BytesIO(f.png)).convert('RGB')
            image.putpixel((0,0),(255,0,0)); output = io.BytesIO()
            image.save(output,format='PNG',compress_level=0)
            async def next_image(): f.png = output.getvalue()
            f.call(next_image())
            before = self.shots()
            a.click_text('⟳ 更新','.preview button')
            wait(lambda:self.shots()>before and self.rendered()
                 and a.evaluate('document.querySelector("img.shot").src')!=previous_src,25)
            # A detached work-b is reopened through the actual chooser.
            a.tap('to-list'); a.tap('runtime-shell-'+a.profiles()[0]['id'])
            index = wait(lambda:a.evaluate('[...document.querySelectorAll(".runtime-sessions li")].findIndex(e=>e.querySelector("strong").textContent.trim()==="work-b")+1'))
            a.click(f'.runtime-sessions li:nth-child({index}) button')
            wait(lambda:a.text('.tab.active .name')=='work-b')
            require(not self.rendered(),'preview crossed tabs')
            self.tab('work-a')
            return {'decodedPNG':True,'firstSeconds':elapsed,'intervalSeconds':interval,'manualNewImageDecoded':True,'tabIsolation':True}
        finally:
            self.tab('work-a')
            if a.evaluate('document.querySelector("[data-testid=desktop-toggle]")?.getAttribute("aria-pressed")==="true"'):
                a.tap('desktop-toggle')
            wait(lambda:not any(n.startswith('mcshot-') for n in f.snapshot()['sessions']))
            state = f.snapshot()
            require(all('preview.py' not in value['input'] for name,value in state['sessions'].items() if name.startswith('work-')),'preview polluted work Shell')
            require(not any(e['method']=='DELETE' and not str(e['name']).startswith('mcshot-') for e in state['events']),'preview deleted a work Shell')

    def lifecycle(self):
        # This independent flow starts with desktop capture off even if the
        # previous preview case failed, without changing its recorded result.
        self.tab('work-a')
        if self.app.evaluate('document.querySelector("[data-testid=desktop-toggle]")?.getAttribute("aria-pressed")==="true"'):
            self.app.tap('desktop-toggle')
        return super().lifecycle()

    def last_tab(self):
        a = self.app
        while a.has('.tab.active button.close'):
            a.click('.tab.active button.close')
        wait(lambda:a.has('[data-testid=no-tabs]'))
        a.tap('no-tabs-targets')
        wait(lambda:a.evaluate('document.querySelector("[data-testid=runtime-host-manage]").getBoundingClientRect().width>0'))
        require('work-a' in self.fixture.snapshot()['sessions'],'closing last tab deleted remote Shell')

    def metadata(self):
        a = self.app
        a.manage(); a.tap('runtime-export')
        # Empty selection, empty/mismatched password all prevent picker.
        require(a.evaluate('document.querySelector(".runtime-transfer button[type=submit]").disabled'),'unselected export enabled')
        a.click('.runtime-transfer fieldset input[type=checkbox]')
        a.set('transfer-password','export-password'); a.set('transfer-confirm','different')
        require(a.evaluate('document.querySelector(".runtime-transfer button[type=submit]").disabled'),'mismatch export enabled')
        a.click('.runtime-transfer form > .choice input')
        require(not a.has('[data-testid=transfer-password]'),'metadata export requires password')
        a.click_text('選擇位置並儲存','.runtime-transfer button')
        name = self.run.name[4:]+'-metadata.json'
        self.device.save_document(name)
        wait(lambda:'連線檔案已儲存' in a.text('[data-testid=runtime-transfer]'),30)
        target = self.run/name
        self.device.call('pull','/sdcard/Download/'+name,str(target))
        raw = target.read_text()
        require(json.loads(raw)['mode']!='encrypted' and self.fixture.credential not in raw,'metadata export contains credential')
        a.click_text('返回管理','.runtime-transfer button')
        a.click_text('刪除','[data-host-id] button'); a.tap('confirm-delete'); wait(lambda:a.profiles()==[])
        a.tap('runtime-import'); a.tap('transfer-pick'); self.device.open_document(name)
        wait(lambda:a.has('.preview-list input[type=checkbox]'))
        require(a.evaluate('document.querySelector("[data-testid=transfer-apply]").disabled'),'metadata import defaults to write')
        a.click('.preview-list input[type=checkbox]')
        require(a.evaluate('document.querySelector("[data-testid=transfer-apply]").disabled'),'missing credential permitted import')
        a.fill('.preview-list input[type=password]',self.fixture.credential)
        a.tap('transfer-apply'); wait(lambda:len(a.profiles())==1,30)
        a.click_text('返回管理','.runtime-transfer button'); self.test_host()
        self.close_manage()

    def tabs(self):
        self.shell('work-b'); self.compose('SECOND_TAB')
        wait(lambda: self.contains('work-b','SECOND_TAB'))
        require(not self.contains('work-a','SECOND_TAB') and not self.contains('work-b','ENTER_繁體'), 'tab outputs crossed')
        self.tab('work-a')

    def tab_target(self):
        a = self.app
        try:
            wait(lambda:a.text('.tab.active .name').strip()=='work-a'
                 and a.text('[data-testid=remote-session]').strip()=='openab:work-a',5)
        except RuntimeError:
            raise CaseFailure('Selected tab and remote-session label disagree',
                              {'activeTab':a.text('.tab.active .name').strip(),
                               'remoteLabel':a.text('[data-testid=remote-session]').strip()}) from None
        self.compose('ACTIVE_A_TARGET_CHECK')
        wait(lambda:'ACTIVE_A_TARGET_CHECK\r' in self.fixture.snapshot()['sessions']['work-a']['input'])
        require('ACTIVE_A_TARGET_CHECK' not in self.fixture.snapshot()['sessions']['work-b']['input'],'active tab input crossed to other Shell')
        return {'activeTab':'work-a','remoteLabel':'openab:work-a','inputOnlyToSelectedShell':True}

    def rotation(self):
        a = self.app; d = self.device; f = self.fixture
        old = f.snapshot()['sessions']['work-a']['resizes']
        d.call('shell','settings','put','system','accelerometer_rotation','0')
        try:
            d.call('shell','settings','put','system','user_rotation','1')
            wait(lambda:a.evaluate('innerWidth>innerHeight') and f.snapshot()['sessions']['work-a']['resizes']!=old)
        finally:
            d.call('shell','settings','put','system','user_rotation','0')
        wait(lambda:a.evaluate('innerHeight>innerWidth'))

    def history_test(self):
        return ReconnectTest(self.app,self.device,self.run,'work-a','matrix')

    def history_seed(self):
        data = ''.join(f'matrix-file-{i:04d}  synthetic ls -al output\r\n' for i in range(1,181))+'READY_MATRIX\r\n'
        self.fixture.call(self.fixture.output(self.fixture.sessions['work-a'],data.encode()))
        wait(lambda:self.history_test().has_old_content() and 'READY_MATRIX' in self.history_test().observed_history())
        return self.history_test().scroll('history-before')

    def recovery(self):
        f = self.fixture; before = f.snapshot()['sessions']['work-a']
        f.disconnect('work-a')
        wait(lambda: len(f.snapshot()['sessions']['work-a']['attaches']) > len(before['attaches']),30)
        wait(lambda:self.history_test().live())
        after = f.snapshot()['sessions']['work-a']
        require(after['attaches'][-1]['tokenHash']==before['attaches'][-1]['tokenHash'], 'recovery changed grant')
        require(after['attaches'][-1]['since']==before['historyBytes'], 'recovery cursor wrong')
        require(after['input']==before['input'], 'recovery replayed input')
        return self.history_test().scroll('history-recovery')

    def ime(self):
        dump = self.device.text('shell','dumpsys','input_method')
        matches = re.findall(r'mInputShown=(true|false)',dump)
        require(matches and len(set(matches))==1, 'IME state unavailable')
        return matches[0]=='true'

    def keyboard_keys(self):
        a = self.app; d = self.device
        d.call('shell','settings','put','secure','show_ime_with_hard_keyboard','1')
        evidence = []
        try:
            for visible in [False,True]:
                for testid in ['k-left','k-up','k-down','k-right']:
                    if self.ime()!=visible:
                        if visible: a.tap('k-keyboard')
                        else: d.key('KEYCODE_BACK')
                    wait(lambda:self.ime()==visible)
                    a.tap(testid); time.sleep(.3)
                    observed = self.ime()
                    evidence.append({'key':testid,'initialVisible':visible,'actualVisible':observed,'preserved':observed==visible})
        finally:
            if self.ime(): d.key('KEYCODE_BACK'); wait(lambda:not self.ime())
            d.call('shell','settings','put','secure','show_ime_with_hard_keyboard','0')
        if not all(e['preserved'] for e in evidence):
            raise CaseFailure('Quick keys changed Android IME visibility',evidence)
        return evidence

    def detach(self):
        return self.history_test().detach_reconnect()

    def close_tab(self):
        self.tab('work-b'); self.app.click('.tab.active button.close')
        wait(lambda:not self.fixture.snapshot()['sessions']['work-b']['attached'])
        require('work-b' in self.fixture.snapshot()['sessions'], 'close deleted remote Shell')
        self.tab('work-a')


def execute(s, focus=None):
    if focus:
        if not s.case('HOST-03','UI connection setup, CRUD and restart',s.crud): return
        if not s.case('SHELL-02','Create fresh work-a through UI',lambda:s.shell('work-a')): return
        if focus == 'history':
            seeded = s.case('READ-02','Fresh single-Shell native scroll with synthetic ls output',s.history_seed)
            if seeded: s.case('READ-05','Append preserves historical reading position',s.append_stays)
            s.case('READ-03','Dropped WSS recovers history and native scrolling',s.recovery)
            s.case('READ-04','Detach/reconnect same Shell retains scrollable history',s.detach)
            s.case('OVERLAY-01','Composer/management hide native reader',s.modal_reader)
            s.case('VOICE-02','Unavailable recognizer has specific feedback',s.voice_unavailable)
        elif focus == 'navigation':
            if not s.case('READ-02','Fresh single-Shell native scroll with synthetic ls output',s.history_seed): return
            s.case('SHELL-03','Shell list refresh/conflict/return',s.shell_listing)
            s.case('TAB-01','Second Shell and isolated tabs',s.tabs)
            s.case('TAB-04','Active tab label and composer destination agree',s.tab_target)
            s.case('READ-06','Native historical reading survives list and tab navigation',lambda:s.history_test().scroll('after-navigation'))
        else:
            s.case('VOICE-01','Voice settings swap while native reader remains visible',s.voice_swap)
            s.case('VOICE-02','Unavailable recognizer has specific feedback',s.voice_unavailable)
            s.case('OVERLAY-01','Composer/management hide native reader',s.modal_reader)
            s.case('VOICE-03','Settings persist across APK restart',s.voice_restart)
        return
    s.case('HOME-01','Empty home, refresh, empty export disabled',s.home_empty)
    s.case('HOST-01','Cancel new connection without writing',s.cancel_host)
    s.case('HOST-02','Invalid connection equivalence classes and UTF8 boundary',s.invalid_hosts)
    # Fresh group setup prevents negative-form failures cascading into CRUD.
    s.reset()
    ready = s.case('HOST-03','Create/test/edit/category/restart/delete last/Keystore',s.crud)
    if ready:
        s.case('HOST-04','Cancel deletion preserves profile',s.delete_cancel)
        s.case('HOST-05','Rejected credential shows failure, correction succeeds',s.auth_error)
        s.case('HOME-02','All/Host/Agent filtering',s.filters)
        s.case('SHELL-01','Invalid Shell names rejected; maxlength32 enforced',s.shell_validation)
        ready = s.case('SHELL-02','Create and connect work-a through UI',lambda:s.shell('work-a'))
    if ready:
        s.case('INPUT-01','Cancel composer does not send',s.compose_cancel)
        s.case('INPUT-02','Send Unicode without Enter',s.compose_plain)
        s.case('INPUT-03','Send Unicode with exactly one Enter and native text',s.compose_enter)
        for suffix,testid,value in [('ESC','k-esc','\x1b'),('TAB','k-tab','\t'),('LEFT','k-left','\x1b[D'),('UP','k-up','\x1b[A'),('DOWN','k-down','\x1b[B'),('RIGHT','k-right','\x1b[C'),('SHIFTLEFT','k-shiftleft','\x1b[1;2D'),('CTRLC','k-ctrlc','\x03')]:
            s.case('KEY-'+suffix,'Touch '+testid+' sends expected bytes',lambda t=testid,v=value:s.key_bytes(t,v))
        s.case('INPUT-04','Ctrl modifier sends control byte; toggle clears latch',s.ctrl)
        s.case('INPUT-05','Configured CLI preserves argument boundaries',s.cli)
        s.case('VOICE-01','Gear swaps preview/radios; native reader stays visible',s.voice_swap)
        s.case('VOICE-02','Missing recognition service has disabled mic and feedback',s.voice_unavailable)
        s.case('OVERLAY-01','Composer and management remain accessible above reader',s.modal_reader)
        history = s.case('READ-02','Large synthetic ls output scrolls without remote input',s.history_seed)
        if history:
            s.case('READ-05','Incoming output preserves historical reading position',s.append_stays)
            s.case('READ-03','Dropped WSS recovers cursor and scrollable old history',s.recovery)
            s.case('READ-04','Detach/reconnect same Shell retains scrollable history',s.detach)
        s.case('SHELL-03','Refresh/attached warning/duplicate conflict/return/primary entry',s.shell_listing)
        s.case('TAB-01','Second Shell and tab switching isolate output',s.tabs)
        s.case('TAB-04','Active tab label and composer destination agree',s.tab_target)
        if history:
            s.case('READ-06','Native historical reading survives list and tab navigation',lambda:s.history_test().scroll('after-navigation'))
        s.case('KEY-IME','Arrows preserve hidden and visible Android keyboard',s.keyboard_keys)
        s.case('READ-01','Portrait/landscape resizes live PTY',s.rotation)
        s.case('TAB-02','Close tab detaches, keeps remote Shell and other tab',s.close_tab)
        s.case('PREVIEW-02','Hide/show image and capture controls without disabling desktop',s.preview_visibility)
        s.case('PREVIEW-01','Decoded PNG, real30s ticks, manual update, tab isolation, cleanup',s.preview)
        s.case('LIFE-01','Home/resume clears draft/Ctrl and pauses preview for33s',s.lifecycle)
        s.case('VOICE-03','Voice modes and language option survive APK restart',s.voice_restart)
        s.case('TAB-03','Close last tab and return to connection list',s.last_tab)
    # Independent SAF group can run even if terminal/preview failed.
    s.reset()
    transfer_ready = s.case('SETUP-TRANSFER','Create dedicated transfer profile',lambda:(s.app.manage(),s.add()))
    if transfer_ready:
        s.case('SAF-01','Real document picker, encryption/cancel/badpassword/duplicate/restart',s.saf)
        s.case('UPDATE-01','Same APK reinstall preserves credential and profile',s.update)
        s.case('SAF-02','Metadata export, password mismatch, missing credential supplement',s.metadata)
    s.blocked('VOICE-RECOGNITION','Tap/hold speech, permission denial, partial/final/cancel/download','API34 default AVD has no speech recognition service; UI settings tested separately')
    s.blocked('X-TRANSFER','Windows APK bidirectional transfer','Linux runner; existing Windows suite retained')
    s.blocked('DEVICE-ARM64','Release signature upgrade/OEM IME/Doze','Requires corresponding physical device and release upgrade fixture')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sdk',type=Path,default=Path('/home/art/Android/Sdk'))
    parser.add_argument('--apk',type=Path,required=True)
    parser.add_argument('--focus',choices=['history','voice','navigation'],help='Run a declared subset plus UI prerequisites')
    args = parser.parse_args()
    require(args.apk.is_file(),'APK missing')
    badging = subprocess.check_output([str(args.sdk/'build-tools/35.0.0/aapt'),'dump','badging',str(args.apk)],text=True)
    require('application-debuggable' in badging and "native-code: 'x86_64'" in badging,'debug x86_64 APK required')
    run = ROOT/'target/android-regression'/('run-ui-'+uuid.uuid4().hex[:12])
    d = LinuxEmulator(args.sdk,run)
    f = s = None
    result = {'createdUtc':datetime.now(timezone.utc).isoformat(),'apkSha256':hashlib.sha256(args.apk.read_bytes()).hexdigest(),
              'scope':args.focus or 'full','actualAPK':True,'backend':'existing TLS/WSS Fixture','mockIPC':False,'cleanup':{}}
    result['sourceHead'] = subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip()
    result['sourceDiffSha256'] = hashlib.sha256(subprocess.check_output(['git','diff','--binary','HEAD'],cwd=ROOT)).hexdigest()
    result['runnerSha256'] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    result['apkVersion'] = re.search(r"versionName='([^']+)'",badging).group(1)
    timer = threading.Timer(1100, _thread.interrupt_main)
    timer.daemon = True; timer.start()
    print('RESULT_DIR '+str(run),flush=True)
    try:
        d.start_emulator(); d.install(args.apk)
        f = Fixture(run); d.reverse(int(f.origin.rsplit(':',1)[1]))
        selected = None
        if args.focus:
            selected = ['HOST-03','SHELL-02','VOICE-02','OVERLAY-01'] + (['READ-02','READ-03','READ-04','READ-05'] if args.focus=='history' else ['VOICE-01','VOICE-03'])
            if args.focus == 'navigation': selected = ['HOST-03','SHELL-02','READ-02','SHELL-03','TAB-01','TAB-04','READ-06']
        s = Matrix(d,f,args.apk,run,selected); s.reset()
        execute(s, args.focus)
    except (Exception, KeyboardInterrupt) as error:
        result['runnerFailure'] = {'type':type(error).__name__,'message':str(error)[:250]}
    finally:
        timer.cancel()
        if s:
            try:
                if s.app:s.app.close()
                result['cleanup']['adb']=d.cleanup()
            except Exception as e:result['cleanup']['adbFailure']=type(e).__name__
        if f:
            try:f.close();result['cleanup']['fixtureStopped']=True
            except Exception as e:result['cleanup']['fixtureFailure']=type(e).__name__
        try:d.shutdown();result['cleanup']['emulatorStopped']=True
        except Exception as e:result['cleanup']['emulatorFailure']=type(e).__name__
        result['cases'] = s.results if s else {}
        result['counts'] = {status:sum(r['status']==status for r in result['cases'].values()) for status in ['PASS','FAIL','BLOCKED','NOT_RUN']}
        result['passed'] = (not result.get('runnerFailure') and result['counts']['FAIL']==0
                            and result['counts']['NOT_RUN']==0 and result['counts']['PASS']>0
                            and not any(k.endswith('Failure') for k in result['cleanup']))
        (run/'result.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
        print(json.dumps({'result':str(run/'result.json'),'counts':result['counts'],'runnerFailure':result.get('runnerFailure')},ensure_ascii=False),flush=True)
    return 0 if result['passed'] else 1


if __name__=='__main__':
    raise SystemExit(main())
