"""Bounded local Windows native regression; no private hosts or mocked IPC."""
import argparse
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import sys
import time
import tomllib
import uuid
from fixture import Fixture
from mcp_fixture import McpFixture
from webview import App, HIDDEN, require, sha
from selenium.webdriver.common.by import By
from selenium.webdriver.common.keys import Keys
from selenium.webdriver.support.ui import Select
from selenium.webdriver.common.action_chains import ActionChains
from selenium.webdriver.common.actions.wheel_input import ScrollOrigin

ROOT = Path(__file__).resolve().parents[2]


def safe_output(value):
    allowed = ROOT / 'target' / 'windows-regression'
    raw = Path(value).absolute()
    require(raw.resolve().is_relative_to(allowed), 'output root outside target/windows-regression')
    for parent in [raw, *raw.parents]:
        require(not parent.is_symlink() and not parent.is_junction(), 'output path contains reparse point')
        if parent == ROOT:
            break
    raw.mkdir(parents=True, exist_ok=True)
    return raw.resolve()


def add(app, fixture, name='Regression runtime'):
    app.manage()
    app.fill('name', name)
    app.fill('base-url', fixture.origin)
    app.fill('credential', fixture.credential)
    app.click('儲存', '[data-testid=runtime-host-form] button')
    app.wait(lambda: len(app.profiles()) == 1)
    return app.profiles()[0]


def test_host(app):
    app.click('測試', '[data-host-id] button')
    app.wait(lambda: any('已通過管理認證' in e.text for e in app.find('[data-host-id] [role=status]')))


def crud(app, fixture, config):
    app.launch(config)
    require(app.profiles() == [] and not config.exists(), 'missing TOML must remain absent and empty')
    profile = add(app, fixture)
    test_host(app)
    require(fixture.credential not in config.read_text(), 'credential leaked to TOML')
    app.click('編輯', '[data-host-id] button')
    app.fill('name', 'Renamed Agent')
    Select(app.node('runtime-host-category')).select_by_value('agent')
    require(app.node('credential').get_attribute('value') == '', 'edit must not read credential back')
    app.click('儲存', '[data-testid=runtime-host-form] button')
    app.wait(lambda: app.profiles()[0]['revision'] == profile['revision'] + 1)
    renamed = app.profiles()[0]
    require(renamed['id'] == profile['id'] and renamed['category'] == 'agent', 'edit identity/category mismatch')
    app.close()
    document = tomllib.loads(config.read_text(encoding='utf-8-sig'))
    require(document['schema_version'] in (5, 6) and bool(document['connections'][0]['credential_ciphertext']), 'DPAPI persistence absent')
    app.launch(config)
    require(app.profiles() == [renamed], 'restart changed saved profile')
    app.manage()
    test_host(app)
    app.click('刪除', '[data-host-id] button')
    app.node('confirm-delete').click()
    app.wait(lambda: app.profiles() == [])
    app.close()
    app.launch(config)
    require(app.profiles() == [], 'deleted last profile reappeared')
    add(app, fixture)
    app.close()
    # Public capability metadata is an existing supported TOML input, not new UI.
    text = config.read_text(encoding='utf-8-sig')
    text = text.replace('category = "host"', 'category = "host"\ndesktop_transport = "runtime-mcp"\ncoding_cli = { id = "fixture-cli", name = "Fixture CLI", command = "fixture-cli", args = ["--literal", "a b"] }')
    config.write_text(text, encoding='utf-8')
    app.launch(config)
    return {'missingFileUnchanged': True, 'uiAddTestEditDelete': True, 'dpapiRestart': True, 'lastDeleteRestartEmpty': True}


def label(name):
    return 's-' + name[4:12] if name.startswith('th2-') else name


def inline_scroll(app, fixture):
    app.main_page()
    require(not app.find('[data-testid=desktop-main], [data-testid=desktop-connect], [data-testid=open-desktop-history]'), 'obsolete workspace/history navigation remains')
    layout = app.driver.execute_script("const r=s=>document.querySelector(s).getBoundingClientRect();const l=r('.desktop-sidebar'),t=r('.desktop-connect-page'),d=r('.desktop-main-page'),p=r('.desktop-preview');return {width:l.width,right:t.left>=l.right-1,below:p.top>=d.bottom-1,vertical:getComputedStyle(document.querySelector('.dashboard')).flexDirection==='column'};")
    require(abs(layout['width'] - 200) <= 1 and layout['right'] and layout['below'] and layout['vertical'], 'workspace layout is not fixed200/stacked/preview-below/right-terminal')
    name = create_shell(app, fixture, app.profiles()[0])
    fixture.append_lines(name, 1, 1800, wide=True)
    css = '.pane .desktop-history .history-viewport'
    viewport = app.wait(lambda: next((e for e in app.find(css) if e.is_displayed() and 'ROW-1800' in e.text), None), 30)
    require(app.driver.execute_script("const h=arguments[0];return h.scrollWidth<=h.clientWidth && [...h.querySelectorAll('.history-line')].some(e=>e.offsetHeight>17);", viewport), 'native long rows do not wrap without horizontal overflow')
    app.wait(lambda: sum(e['method'] == 'history' and e['name'] == name and e['action'] == 'older' for e in fixture.snapshot()['events']) >= 3)
    def ids():
        return app.driver.execute_script("return [...arguments[0].querySelectorAll('[data-line-id]')].map(e=>Number(e.dataset.lineId));", viewport)
    def wheel(amount):
        ActionChains(app.driver).scroll_from_origin(ScrollOrigin.from_element(viewport), 0, amount).perform()
    original = ids()[0]
    wheel(-10000)
    app.wait(lambda: ids()[0] < original)
    anchored = app.driver.execute_script("const h=arguments[0];return h.querySelector('[data-line-id=\"'+arguments[1]+'\"]').getBoundingClientRect().top-h.getBoundingClientRect().top;", viewport, original)
    require(abs(anchored - 4) <= 2, f'older page moved the reading anchor ({anchored:.2f}px)')
    divider = app.node('sidebar-divider')
    ActionChains(app.driver).move_to_element(divider).click_and_hold().move_by_offset(120, 0).release().perform()
    app.wait(lambda: app.driver.execute_script("return Math.abs(document.querySelector('.desktop-sidebar').getBoundingClientRect().width-320)<2"))
    app.wait(lambda: app.driver.execute_script("const h=arguments[0],e=h.querySelector('[data-line-id=\"'+arguments[1]+'\"]');return Math.abs(e.getBoundingClientRect().top-h.getBoundingClientRect().top-4)<=2;", viewport, original))
    ActionChains(app.driver).move_to_element(app.node('sidebar-divider')).click_and_hold().move_by_offset(-120, 0).release().perform()
    for _ in range(4):
        first = ids()[0]
        wheel(-10000)
        app.wait(lambda: ids()[0] < first)
    require(len(ids()) == 1024, 'history window did not remain bounded at 1024 rows')
    held = app.driver.execute_script('return arguments[0].scrollTop;', viewport)
    fixture.append_lines(name, 1801, 25)
    app.driver.execute_async_script('const done=arguments[0];setTimeout(done,750);')
    require('ROW-1825' not in viewport.text and abs(app.driver.execute_script('return arguments[0].scrollTop;', viewport) - held) <= 1, 'new output moved the reading position')
    for _ in range(10):
        if 'ROW-1825' in viewport.text:
            break
        last = ids()[-1]
        wheel(100000)
        app.wait(lambda: ids()[-1] > last or 'ROW-1825' in viewport.text)
    require('ROW-1825' in viewport.text, 'scrolling down did not resume latest output')
    viewport.find_elements(By.CSS_SELECTOR, '.history-line')[-1].click()
    ActionChains(app.driver).send_keys('INLINE_KEYBOARD' + Keys.ENTER).perform()
    app.wait(lambda: app.contains(name, 'INLINE_KEYBOARD'))
    app.wait(lambda: 'INLINE_KEYBOARD' in viewport.text)
    app.driver.save_screenshot(str(app.run / 'workspace-inline.png'))
    events = [e for e in fixture.snapshot()['events'] if e['method'] == 'history' and e['name'] == name]
    require(any(e['action'] == 'ack' for e in events) and sum(e['action'] == 'older' for e in events) >= 5, 'native history paging/ACK absent')
    app.main_page()
    app.node('desktop-settings').click()
    app.main_page()
    return {'directWheelPaging': True, 'prefetchedThreePages': True, 'wrappedWithoutHorizontalOverflow': True, 'nativeDragKeepsAnchor': True, 'bounded1024': True, 'anchorPixelError': abs(anchored - 4), 'heldDuringNewOutput': True, 'bottomFollowsLatest': True, 'clickAndRealKeyboard': True, 'singleWorkspaceLayout': layout}


def create_shell(app, fixture, profile):
    app.main_page()
    before = set(fixture.snapshot()['sessions'])
    css = '[data-testid="runtime-' + profile['id'] + '"] .new-connection'
    app.wait(lambda: next((e for e in app.find(css) if e.is_enabled() and e.is_displayed()), None)).click()
    def created():
        return next((name for name in fixture.snapshot()['sessions'] if name not in before and name.startswith('th2-')), None)
    name = app.wait(created)
    app.wait(lambda: any(e.text == label(name) for e in app.find('.tab .name')))
    app.wait(lambda: any(e.is_displayed() for e in app.find('.toolbar .status.live')))
    app.wait(lambda: fixture.snapshot()['sessions'][name]['bootstrap'])
    return name


def active_tab(app, name):
    app.connect_page()
    tab = app.wait(lambda: next((e for e in app.find('.tab') if e.find_element(By.CSS_SELECTOR, '.name').text == label(name)), None))
    tab.click()
    return tab


def terminal(app, fixture):
    # Management is a modal above Settings, close it before Main navigation.
    for element in app.find('.runtime-host-backdrop > .runtime-hosts > header button'):
        if element.is_displayed() and element.text == '關閉': element.click()
    profile = app.profiles()[0]
    work_a = create_shell(app, fixture, profile)
    app.terminal('UNICODE_繁體測試' + Keys.ENTER)
    app.wait(lambda: app.contains(work_a, 'UNICODE_繁體測試'))
    app.wait(lambda: app.driver.execute_script('return window.__reg.acks>0;'))
    require(app.driver.execute_script("return [...document.querySelectorAll('.xterm-rows')].some(e=>e.textContent.includes('UNICODE_繁體測試'))"), 'Unicode not rendered by real xterm')
    app.node('launch-coding-cli').click()
    app.wait(lambda: "'fixture-cli' '--literal' 'a b'\r" in fixture.snapshot()['sessions'][work_a]['input'])
    work_b = create_shell(app, fixture, profile)
    app.terminal('SECOND_TAB' + Keys.ENTER)
    app.wait(lambda: app.contains(work_b, 'SECOND_TAB'))
    require(not app.contains(work_a, 'SECOND_TAB') and not app.contains(work_b, 'UNICODE_繁體測試'), 'tab outputs crossed bindings')
    active_tab(app, work_a)
    before_size = fixture.snapshot()['sessions'][work_a]['resizes']
    app.driver.set_window_size(1090, 760)
    app.wait(lambda: fixture.snapshot()['sessions'][work_a]['resizes'] != before_size)
    app.wait(lambda: app.driver.execute_script('return window.__reg.acks>2;'))
    before = fixture.snapshot()
    require(len(before['sessions'][work_a]['attaches']) == 1, 'initial attach count mismatch')
    fixture.disconnect(work_a)
    app.wait(lambda: len(fixture.snapshot()['sessions'][work_a]['attaches']) == 2, 25)
    after = fixture.snapshot()
    first, resumed = after['sessions'][work_a]['attaches']
    require(first['tokenHash'] == resumed['tokenHash'], 'recovery minted another token')
    require(resumed['since'] == before['sessions'][work_a]['historyBytes'] and resumed['since'] > 0, 'recovery did not use rendered cursor')
    require(after['sessions'][work_a]['input'] == before['sessions'][work_a]['input'], 'recovery replayed input')
    # Reader helpers may reopen; the work Shell must keep its original binding.
    mutations = lambda state: [e for e in state['events'] if e['method'] in ('POST', 'DELETE', 'bootstrap') and not (e.get('name') or '').startswith('mcshot-')]
    require(mutations(after) == mutations(before), 'recovery created/renewed/deleted/bootstrap work session')
    app.terminal('AFTER_RECOVERY' + Keys.ENTER)
    app.wait(lambda: app.contains(work_a, 'AFTER_RECOVERY'))
    active_tab(app, work_b).find_element(By.CSS_SELECTOR, 'button.close').click()
    app.wait(lambda: not fixture.snapshot()['sessions'][work_b]['attached'])
    app.main_page()
    root = '[data-testid="runtime-' + profile['id'] + '"]'
    app.wait(lambda: [e for e in app.find(root + ' .host-identity') if e.is_enabled()])[0].click()
    def attach():
        return next((e for e in app.find(root + ' .attach-button') if label(work_b) in e.text and e.is_displayed() and e.is_enabled()), None)
    # Multiple sessions initially use the expandable count button.
    if not attach():
        app.wait(lambda: next((e for e in app.find(root + ' .connection-summary') if e.is_displayed()), None)).click()
    app.wait(attach).click()
    app.wait(lambda: len(fixture.snapshot()['sessions'][work_b]['attaches']) == 2)
    renewed = [e for e in fixture.snapshot()['events'] if e.get('action') == 'renew']
    require(len(renewed) == 1 and renewed[0]['name'] == work_b, 'explicit selection did not renew exactly once')
    require(sum(e['method'] == 'bootstrap' and e['name'] == work_b for e in fixture.snapshot()['events']) == 1, 'reattach repeated bootstrap')
    app.work_a, app.work_b = work_a, work_b
    active_tab(app, work_a)
    return {'realXtermUnicode': True, 'tabsIsolated': True, 'nativeResize': True,
            'sameTokenSinceRecovery': True, 'noInputReplay': True, 'explicitSelectionRenew': True, 'closeDetachesOnly': True, 'bootstrapExactlyOnce': True}


def preview(app, fixture, config, screens):
    app.settings()
    for upstream in screens:
        app.fill('screen-name', 'Screen ' + upstream.name)
        app.fill('screen-url', upstream.url)
        app.fill('screen-token', upstream.token)
        app.click('儲存 Screen', '[data-testid=screen-form] button')
        app.wait(lambda: any(d['name'] == 'Screen ' + upstream.name for d in app.screens()))
        app.wait(lambda: app.node('screen-name').is_enabled())
        require(app.node('screen-token').get_attribute('value') == '', 'saved Screen token retained')
    saved = app.screens()
    require(len(saved) == 2 and not any(u.events for u in screens), 'Screen catalog contacted upstream')
    raw = config.read_text(encoding='utf-8-sig')
    document = tomllib.loads(raw)
    require(document['schema_version'] == 6 and len(document['connections']) == 1 and len(document['desktops']) == 2, 'Screen persistence lost Host')
    require(all(d.get('token_ciphertext') for d in document['desktops']) and all(u.token not in raw for u in screens), 'Screen DPAPI ciphertext missing or plaintext leaked')
    # Real restart reads persisted DPAPI Screen tokens.
    app.close(); app.launch(config)
    require(app.screens() == saved, 'Screen metadata changed across restart')
    app.settings()
    first = saved[0]
    row = '[data-testid="saved-screen-' + first['id'] + '"]'
    app.click('編輯', row + ' button')
    require(app.node('screen-token').get_attribute('value') == '', 'edit read back Screen token')
    app.fill('screen-name', first['name'] + ' edited')
    app.click('儲存 Screen', '[data-testid=screen-form] button')
    app.wait(lambda: next(d for d in app.screens() if d['id'] == first['id'])['revision'] == first['revision'] + 1)
    app.wait(lambda: app.node('screen-name').is_enabled())
    app.click('測試', row + ' button')
    app.wait(lambda: any('Screen 連線測試通過' in e.text for e in app.find('[data-testid=screen-settings] [role=status]')))
    profile = app.profiles()[0]
    work_a = create_shell(app, fixture, profile)
    work_b = create_shell(app, fixture, profile)
    app.work_a, app.work_b = work_a, work_b
    active_tab(app, work_a)
    saved = app.screens()
    by_name = {d['name'].split()[1]: d for d in saved}
    def choose(upstream):
        app.node('desktop-toggle').click()
        app.node('screen-picker')
        app.node('choose-screen-' + by_name[upstream.name]['id']).click()
        expected = 'data:image/png;base64,' + __import__('base64').b64encode(upstream.image).decode()
        app.wait(lambda: app.rendered(expected), 30)
        require(not any(e.is_displayed() for e in app.find('[data-testid=screen-picker]')), 'Screen picker remained open')
        require(any(e['method'] == 'tools/call' for e in upstream.events), 'direct native MCP screenshot absent')
        return expected
    choose(screens[0])
    image_fits = app.driver.execute_script("const p=document.querySelector('.desktop-preview').getBoundingClientRect(),i=document.querySelector('img.shot').getBoundingClientRect();return i.top>=p.top && i.bottom<=p.bottom && i.left>=p.left && i.right<=p.right;")
    require(image_fits, 'desktop image is clipped outside its sidebar preview')
    app.driver.save_screenshot(str(app.run / 'workspace-screen.png'))
    before = app.shots()
    before_creates = sum(e['method'] == 'POST' and e.get('action') is None for e in fixture.snapshot()['events'])
    app.click('⟳ 更新', '.preview button')
    app.wait(lambda: app.shots() > before and app.rendered(), 25)
    require(sum(e['method'] == 'POST' and e.get('action') is None for e in fixture.snapshot()['events']) == before_creates, 'direct screenshot created a Runtime Shell')
    active_tab(app, work_b)
    require(not app.rendered(), 'Screen image crossed tabs')
    active_tab(app, work_a)
    choose(screens[1])
    app.node('desktop-toggle').click(); app.node('clear-screen').click()
    app.wait(lambda: not app.find('img.shot'))
    app.wait(lambda: all(not u.sessions for u in screens))
    tools_helpers = {e['name'] for e in fixture.snapshot()['events'] if e['method'] == 'tools-control'}
    app.wait(lambda: not tools_helpers.intersection(fixture.snapshot()['sessions']))
    state = fixture.snapshot()
    owned_created = sorted(e['name'] for e in state['events'] if e['method'] == 'POST' and e.get('action') is None and e['name'] in tools_helpers)
    owned_deleted = sorted(e['name'] for e in state['events'] if e['method'] == 'DELETE' and e['name'] in tools_helpers)
    require(owned_created == owned_deleted and owned_created, 'owned configure Shell cleanup mismatch')
    require(any(e['method'] == 'tools-control' and e['action'] == 'configure' for e in state['events']), 'READY configure handshake absent')
    require(any(e['method'] == 'tools-control' and e['action'] == 'clear' for e in state['events']), 'clear handshake absent')
    work = [v for n, v in state['sessions'].items() if n.startswith('th2-')]
    require(all('instance_bridge.py' not in v['input'] and 'ARTTERM_TOOLS' not in v['input'] for v in work), 'tools configuration polluted work Shell')
    app.terminal('AFTER_SCREEN' + Keys.ENTER)
    app.wait(lambda: app.contains(work_a, 'AFTER_SCREEN'))
    # Runtime explicit exit closes the matching tab, retaining its peer.
    fixture.finish(work_b)
    app.wait(lambda: not any(e.text == label(work_b) for e in app.find('.tab .name')))
    require(any(e.text == label(work_a) for e in app.find('.tab .name')), 'exit closed unrelated tab')
    app.settings()
    for desktop in app.screens():
        app.click('刪除', '[data-testid="saved-screen-' + desktop['id'] + '"] button')
        app.wait(lambda: all(d['id'] != desktop['id'] for d in app.screens()))
    app.close(); app.launch(config)
    require(app.screens() == [] and len(app.profiles()) == 1, 'Screen delete/restart lost Host or restored Screen')
    return {'screenDpapiCrudRestart': True, 'directJsonAndSseDecoded': True, 'twoScreensSwitchClear': True,
            'tabImageIsolation': True, 'workShellIsolated': True, 'readyConfigureClear': True,
            'noAiAdapterClaim': True, 'ownedConfigureShellsDeleted': len(owned_deleted), 'noRuntimeScreenshotShell': True, 'exit4004ClosesOnlyMatchingTab': True,
            'historyPanel': 'NOT_RUN: real Host tmux paging/ACK/keep/close requires manual handoff',
            'automatic30SecondCapture': 'NOT_RUN: run manual timed observation on Windows'}


def import_file(app, exported, password):
    app.picker(lambda: app.node('transfer-pick').click(), exported)
    app.fill('transfer-password', password)
    app.click('解鎖並預覽', '.runtime-transfer button')


def transfer(app, fixture, config, destination, exported):
    password = secrets.token_urlsafe(24)
    app.manage()
    app.node('runtime-export').click()
    app.wait(lambda: app.find('.runtime-transfer fieldset input[type=checkbox]'))[0].click()
    app.fill('transfer-password', password)
    app.fill('transfer-confirm', password)
    app.picker(lambda: app.click('選擇位置並儲存', '.runtime-transfer button'), exported)
    app.wait(lambda: exported.exists() and '連線檔案已儲存' in app.node('runtime-transfer').text)
    raw = exported.read_text()
    require(json.loads(raw)['mode'] == 'encrypted' and fixture.credential not in raw and fixture.origin not in raw, 'transfer not fully encrypted')
    app.close()
    app.launch(destination)
    app.manage()
    app.node('runtime-import').click()
    import_file(app, exported, 'wrong-password')
    app.wait(lambda: '密碼錯誤或檔案已損壞' in app.node('runtime-transfer').text, 30)
    require(not destination.exists() and app.profiles() == [], 'wrong password modified destination')
    import_file(app, exported, password)
    app.node('transfer-apply').click()
    app.wait(lambda: len(app.profiles()) == 1, 30)
    restored = app.profiles()[0]
    original = tomllib.loads(config.read_text(encoding='utf-8-sig'))['connections'][0]
    require(restored['id'] != original['id'] and restored['credentialPresent'], 'import did not create local credential identity')
    app.click('返回管理', '.runtime-transfer button')
    test_host(app)
    app.node('runtime-import').click()
    before = destination.read_bytes()
    import_file(app, exported, password)
    checkbox = app.wait(lambda: app.find('.preview-list input[type=checkbox]'))[0]
    require(not checkbox.is_selected() and not app.node('transfer-apply').is_enabled(), 'duplicate did not default to skip')
    require(destination.read_bytes() == before, 'preview default skip wrote destination')
    checkbox.click()
    app.node('transfer-apply').click()
    app.wait(lambda: app.profiles()[0]['revision'] == restored['revision'] + 1, 30)
    app.close()
    app.launch(destination)
    app.manage()
    test_host(app)
    require(app.profiles()[0]['id'] == restored['id'], 'updated import identity did not persist')
    app.close()
    password = ''
    return {'encryptedNativeExport': True, 'ownedNativePicker': True, 'wrongPasswordNoWrite': True,
            'importAndDpapiRestart': True, 'duplicateDefaultSkip': True, 'explicitUpdatePersists': True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--exe', required=True, type=Path)
    parser.add_argument('--driver', required=True, type=Path)
    parser.add_argument('--output-root', default=ROOT / 'target/windows-regression', type=Path)
    args = parser.parse_args()
    run = safe_output(args.output_root) / ('run-' + uuid.uuid4().hex)
    run.mkdir()
    result = {'passed': False, 'actualWindowsExe': False, 'mockedIpcReplies': False, 'mockedUpstream': True, 'realAiCli': False, 'manualNotRun': ['real Host history paging/ACK/keep/close', '30-second timed auto capture', 'real upstream AI adapter/tools/broker'], 'scenarios': {}, 'cleanup': {}}
    fixture = app = None
    screens = []
    stage = 'preflight'
    try:
        require(os.name == 'nt', 'Windows native E2E requires Windows')
        exe, driver = args.exe.resolve(strict=True), args.driver.resolve(strict=True)
        result['actualWindowsExe'] = True
        result['exeSha256'] = sha(exe)
        source_files = sorted([*ROOT.glob('crates/art-term-core/src/**/*.rs'), *ROOT.glob('crates/art-term-gui/src/**/*.rs'),
                               *ROOT.glob('apps/desktop/src/**/*'), *Path(__file__).parent.glob('*')])
        result['sourceHashes'] = {str(p.relative_to(ROOT)).replace('\\', '/'): sha(p) for p in source_files if p.is_file()}
        version = subprocess.run([str(driver), '--version'], capture_output=True, text=True, timeout=10, creationflags=HIDDEN)
        require(version.returncode == 0 and re.search(r'\d+\.\d+\.\d+\.\d+', version.stdout), 'EdgeDriver version unavailable')
        result['driverVersion'] = version.stdout.strip()
        fixture = Fixture(run)
        screens = [McpFixture('A'), McpFixture('B', sse=True)]
        app = App(exe, driver, run, fixture)
        config, destination = run / 'connections.toml', run / 'imported.toml'
        for name, function in [('crud', lambda: crud(app, fixture, config)),
                               ('terminal', lambda: terminal(app, fixture)),
                               ('inline-scroll', lambda: inline_scroll(app, fixture)),
                               ('screens', lambda: preview(app, fixture, config, screens)),
                               ('transfer', lambda: transfer(app, fixture, config, destination, run / 'transfer.json'))]:
            stage = name
            outcome = function()
            result['scenarios'][name] = {'passed': True, 'checks': outcome}
            print('PASS ' + name, flush=True)
        result['webview'] = app.metadata
        require(re.search(r'\d+\.\d+\.\d+\.\d+', result['driverVersion'])[0].split('.')[:3] == app.metadata['browserVersion'].split('.')[:3], 'EdgeDriver/WebView2 version mismatch')
        result['passed'] = True
    except Exception as error:
        result['failure'] = {'stage': stage, 'type': type(error).__name__}
        # Only our own fixed RuntimeError text; Selenium messages can include arguments/secrets.
        if type(error) is RuntimeError:
            result['failure']['code'] = str(error)
        result['scenarios'].setdefault(stage, {'passed': False})
        if app and app.driver:
            try:
                app.driver.save_screenshot(str(run / 'failure.png'))
            except Exception:
                pass
    finally:
        for upstream in screens:
            try:
                upstream.close()
            except Exception:
                result['passed'] = False
                result['cleanup']['mcpFailure'] = True
        if app:
            try:
                app.close()
                result['cleanup']['app'] = app.cleanups
            except Exception as error:
                result['cleanup']['appFailure'] = type(error).__name__
                result['passed'] = False
        if fixture:
            try:
                state = fixture.snapshot()
                result['fixtureSummary'] = {'sessions': {n: {k: v for k, v in s.items() if k != 'input'} for n, s in state['sessions'].items()},
                                            'events': state['events'], 'pngBytes': state['pngBytes']}
                require(not any(e['method'] == 'DELETE' and not e['name'].startswith('mcshot-') for e in state['events']), 'work Shell was deleted')
            except Exception as error:
                result['cleanup']['ledgerFailure'] = type(error).__name__
                result['passed'] = False
            try:
                fixture.close()
                result['cleanup']['fixtureStopped'] = True
            except Exception as error:
                result['cleanup']['fixtureFailure'] = type(error).__name__
                result['passed'] = False
        (run / 'result.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
    print(json.dumps({'passed': result['passed'], 'scenariosPassed': sum(s.get('passed', False) for s in result['scenarios'].values()),
                      'result': str(run / 'result.json')}), flush=True)
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
