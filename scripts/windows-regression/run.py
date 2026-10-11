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
from webview import App, HIDDEN, require, sha
from selenium.webdriver.common.by import By
from selenium.webdriver.common.keys import Keys
from selenium.webdriver.support.ui import Select

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
    app.node('runtime-create').click()
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
    require(document['schema_version'] == 5 and bool(document['connections'][0]['credential_ciphertext']), 'schema5 DPAPI persistence absent')
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
    return {'missingFileUnchanged': True, 'uiAddTestEditDelete': True, 'schema5DpapiRestart': True, 'lastDeleteRestartEmpty': True}


def create_shell(app, profile, name):
    app.node('runtime-shell-' + profile['id']).click()
    element = app.wait(lambda: [e for e in app.find('.runtime-sessions form input') if e.is_enabled()])[0]
    element.send_keys(name)
    app.click('建立並連線', '.runtime-sessions form button')
    app.wait(lambda: any(e.text == name for e in app.find('.tab .name')))
    app.wait(lambda: app.find('.toolbar .status.live'))


def active_tab(app, name):
    tab = app.wait(lambda: next((e for e in app.find('.tab') if e.find_element(By.CSS_SELECTOR, '.name').text == name), None))
    tab.click()
    return tab


def terminal(app, fixture):
    profile = app.profiles()[0]
    create_shell(app, profile, 'work-a')
    app.terminal('UNICODE_繁體測試' + Keys.ENTER)
    app.wait(lambda: app.contains('work-a', 'UNICODE_繁體測試'))
    app.wait(lambda: app.driver.execute_script('return window.__reg.acks>0;'))
    require(app.driver.execute_script("return [...document.querySelectorAll('.xterm-rows')].some(e=>e.textContent.includes('UNICODE_繁體測試'))"), 'Unicode not rendered by real xterm')
    app.node('launch-coding-cli').click()
    app.wait(lambda: "'fixture-cli' '--literal' 'a b'\r" in fixture.snapshot()['sessions']['work-a']['input'])
    create_shell(app, profile, 'work-b')
    app.terminal('SECOND_TAB' + Keys.ENTER)
    app.wait(lambda: app.contains('work-b', 'SECOND_TAB'))
    require(not app.contains('work-a', 'SECOND_TAB') and not app.contains('work-b', 'UNICODE_繁體測試'), 'tab outputs crossed bindings')
    active_tab(app, 'work-a')
    before_size = fixture.snapshot()['sessions']['work-a']['resizes']
    app.driver.set_window_size(1090, 760)
    app.wait(lambda: fixture.snapshot()['sessions']['work-a']['resizes'] != before_size)
    before = fixture.snapshot()
    require(len(before['sessions']['work-a']['attaches']) == 1, 'initial attach count mismatch')
    app.wait(lambda: app.driver.execute_script('return window.__reg.acks>2;'))
    fixture.disconnect('work-a')
    app.wait(lambda: len(fixture.snapshot()['sessions']['work-a']['attaches']) == 2, 25)
    after = fixture.snapshot()
    first, resumed = after['sessions']['work-a']['attaches']
    require(first['tokenHash'] == resumed['tokenHash'], 'recovery minted another token')
    require(resumed['since'] == before['sessions']['work-a']['historyBytes'] and resumed['since'] > 0, 'recovery did not use rendered cursor')
    require(after['sessions']['work-a']['input'] == before['sessions']['work-a']['input'], 'recovery replayed input')
    require(after['events'] == before['events'], 'recovery created/renewed/deleted session')
    app.terminal('AFTER_RECOVERY' + Keys.ENTER)
    app.wait(lambda: app.contains('work-a', 'AFTER_RECOVERY'))
    active_tab(app, 'work-b').find_element(By.CSS_SELECTOR, 'button.close').click()
    app.wait(lambda: not fixture.snapshot()['sessions']['work-b']['attached'])
    app.node('runtime-shell-' + profile['id']).click()
    app.wait(lambda: app.find('.runtime-sessions li'))
    for row in app.find('.runtime-sessions li'):
        if row.find_element(By.CSS_SELECTOR, 'strong').text == 'work-b':
            row.find_element(By.CSS_SELECTOR, 'button').click()
            break
    app.wait(lambda: len(fixture.snapshot()['sessions']['work-b']['attaches']) == 2)
    renewed = [e for e in fixture.snapshot()['events'] if e.get('action') == 'renew']
    require(len(renewed) == 1 and renewed[0]['name'] == 'work-b', 'explicit selection did not renew exactly once')
    active_tab(app, 'work-a')
    return {'realXtermUnicode': True, 'tabsIsolated': True, 'nativeResize': True,
            'sameTokenSinceRecovery': True, 'noInputReplay': True, 'explicitSelectionRenew': True, 'closeDetachesOnly': True}


def preview(app, fixture):
    started = time.monotonic()
    app.click('啟用桌面', '.toolbar button')
    app.wait(lambda: app.shots() >= 1 and app.rendered(), 25)
    first = app.shots()
    first_elapsed = time.monotonic() - started
    require(first_elapsed < 20, 'first capture waited for periodic timer')
    # The production interval belongs to the controller, not enable time.
    # Observe two untouched ticks to measure its actual 30s interval.
    app.wait(lambda: app.shots() >= first + 2 and app.rendered(), 75)
    automatic_elapsed = time.monotonic() - started
    starts = app.driver.execute_script('return window.__reg.shotStarts;')
    automatic_interval = (starts[-1] - starts[-2]) / 1000
    require(24 <= automatic_interval <= 38, 'auto refresh did not use production interval')
    automatic = app.shots()
    app.click('⟳ 更新', '.preview button')
    app.wait(lambda: app.shots() > automatic and app.rendered(), 25)
    active_tab(app, 'work-b')
    require(not app.rendered(), 'inactive tab image crossed into other tab')
    active_tab(app, 'work-a')
    app.click('停用桌面', '.toolbar button')
    app.wait(lambda: not app.find('img.shot'))
    app.terminal('AFTER_PREVIEW' + Keys.ENTER)
    app.wait(lambda: app.contains('work-a', 'AFTER_PREVIEW'))
    app.wait(lambda: not any(n.startswith('mcshot-') for n in fixture.snapshot()['sessions']))
    state = fixture.snapshot()
    work = [s for n, s in state['sessions'].items() if n.startswith('work-')]
    require(all('preview.py' not in s['input'] and 'ARTTERM_PREVIEW' not in s['input'] for s in work), 'preview polluted work Shell')
    deleted = [e['name'] for e in state['events'] if e['method'] == 'DELETE']
    created = [e['name'] for e in state['events'] if e['method'] == 'POST' and e.get('action') is None and e['name'].startswith('mcshot-')]
    require(deleted and sorted(deleted) == sorted(created), 'preview cleanup ownership mismatch')
    return {'pngBytes': state['pngBytes'], 'realImageDecoded': True, 'firstCaptureSeconds': round(first_elapsed, 2),
            'automaticCaptureSeconds': round(automatic_elapsed, 2), 'automaticIntervalSeconds': round(automatic_interval, 2), 'manual': True, 'disableClears': True,
            'tabImageIsolation': True, 'workShellIsolated': True, 'ownedTemporaryShellsDeleted': len(deleted)}


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
    result = {'passed': False, 'actualWindowsExe': True, 'mockedIpcReplies': False, 'scenarios': {}, 'cleanup': {}}
    fixture = app = None
    stage = 'preflight'
    try:
        require(os.name == 'nt', 'Windows native E2E requires Windows')
        exe, driver = args.exe.resolve(strict=True), args.driver.resolve(strict=True)
        result['exeSha256'] = sha(exe)
        source_files = sorted([*ROOT.glob('crates/art-term-core/src/**/*.rs'), *ROOT.glob('crates/art-term-gui/src/**/*.rs'),
                               *ROOT.glob('apps/desktop/src/**/*'), *Path(__file__).parent.glob('*')])
        result['sourceHashes'] = {str(p.relative_to(ROOT)).replace('\\', '/'): sha(p) for p in source_files if p.is_file()}
        version = subprocess.run([str(driver), '--version'], capture_output=True, text=True, timeout=10, creationflags=HIDDEN)
        require(version.returncode == 0 and re.search(r'\d+\.\d+\.\d+\.\d+', version.stdout), 'EdgeDriver version unavailable')
        result['driverVersion'] = version.stdout.strip()
        fixture = Fixture(run)
        app = App(exe, driver, run, fixture)
        config, destination = run / 'connections.toml', run / 'imported.toml'
        for name, function in [('crud', lambda: crud(app, fixture, config)),
                               ('terminal', lambda: terminal(app, fixture)),
                               ('preview', lambda: preview(app, fixture)),
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
