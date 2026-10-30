"""Focused unified-connection checks in the actual Windows EXE and WebView2."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import tomllib
import uuid
from fixture import Fixture
from mcp_fixture import McpFixture
from webview import App, require, sha
from selenium.webdriver.common.action_chains import ActionChains
from selenium.webdriver.common.keys import Keys
from run import create_shell


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--exe', required=True, type=Path)
    parser.add_argument('--driver', required=True, type=Path)
    parser.add_argument('--source', required=True)
    args = parser.parse_args()
    root = Path(tempfile.gettempdir()) / ('art-term-connections-' + uuid.uuid4().hex)
    root.mkdir(mode=0o700)
    result = {'passed': False, 'source': args.source, 'actualWindowsExe': True,
              'mockedIpcReplies': False, 'mockedUpstream': True, 'checks': {}, 'cleanup': {}}
    fixture = app = None
    screens = []
    stage = 'preflight'
    print('OUTPUT ' + str(root), flush=True)
    try:
        require(os.name == 'nt', 'Windows is required')
        exe = args.exe.resolve(strict=True)
        driver = args.driver.resolve(strict=True)
        result['exeSha256'] = sha(exe)
        fixture = Fixture(root)
        screens = [McpFixture('A'), McpFixture('B')]
        app = App(exe, driver, root, fixture)
        config = root / 'connections.toml'
        app.launch(config)
        app.driver.execute_script("window.__connectionPointers=[];document.addEventListener('pointerdown',e=>{if(e.target.closest('[data-action=reorder]'))window.__connectionPointers.push({trusted:e.isTrusted,type:e.pointerType})},true)")

        def manage():
            app.node('desktop-settings').click()
            app.node('settings-connections').click()
            app.node('runtime-host-list')

        def row(identity):
            return app.wait(lambda: next((e for e in app.find('[data-connection-id="' + identity + '"]') if e.is_displayed()), None))

        def action(identity, name):
            return row(identity).find_element('css selector', '[data-action="' + name + '"]')

        def toast(text):
            app.wait(lambda: text in app.node('connection-toast').text)

        def add(kind, name, url, token, alias=''):
            app.node('runtime-create').click()
            app.node('connection-type-' + kind).click()
            app.fill('name', name)
            app.fill('base-url', url)
            app.fill('credential', token)
            if kind != 'screen':
                app.fill('display-name', alias)
            app.click('儲存', '[data-testid=runtime-host-form] button')
            toast('儲存成功')
            app.wait(lambda: not app.find('[data-testid=runtime-host-form]'))
            values = app.screens() if kind == 'screen' else app.profiles()
            return next(value for value in values if value['name'] == name)

        def order(kind):
            return app.driver.execute_script("return [...document.querySelectorAll('[data-connection-kind=\"'+arguments[0]+'\"]')].map(e=>e.dataset.connectionId)", kind)

        def drag(source, target, after=False):
            src = action(source, 'reorder')
            dst = row(target)
            app.driver.execute_script('arguments[0].scrollIntoView({block:"center"})', dst)
            ActionChains(app.driver).move_to_element(src).click_and_hold().pause(.1).move_to_element_with_offset(dst, 0, dst.size['height'] * (.3 if after else -.3)).pause(.2).release().perform()

        stage = 'unified-create'
        print('STAGE ' + stage, flush=True)
        manage()
        require(not app.find('[data-testid=runtime-host-form]'), 'creation form is initially visible')
        require(not app.find('[data-testid=settings-screens]'), 'separate Screen settings remains')
        computer = add('host', 'Windows computer', fixture.origin, fixture.credential, '開發助手')
        agent_a = add('agent', 'Agent A', 'https://agent-a.example.test', 'owned-agent-a')
        agent_b = add('agent', 'Agent B', 'https://agent-b.example.test', 'owned-agent-b')
        screen_a = add('screen', 'Screen A', screens[0].url, screens[0].token)
        screen_b = add('screen', 'Screen B', screens[1].url, screens[1].token)
        require(computer['displayName'] == '開發助手' and computer['agentTool'] == '開發助手', 'Chinese alias is not returned by native store')
        raw = config.read_bytes()
        require(all(secret.encode() not in raw for secret in (fixture.credential, screens[0].token, screens[1].token, 'owned-agent-a', 'owned-agent-b')), 'plaintext Token persisted')
        document = tomllib.loads(raw.decode('utf-8-sig'))
        require(document['schema_version'] == 6 and all(item['credential_ciphertext'] for item in document['connections']) and all(item['token_ciphertext'] for item in document['desktops']), 'DPAPI ciphertext missing')
        result['checks']['unifiedCreationAndDpapi'] = True
        app.driver.save_screenshot(str(root / 'connections.png'))

        stage = 'tests-and-toast'
        print('STAGE ' + stage, flush=True)
        action(computer['id'], 'test').click()
        toast('連線測試成功')
        require('成功' not in row(computer['id']).text, 'test result remains inside row')
        geometry = app.driver.execute_script("const e=document.querySelector('[data-testid=connection-toast]'),r=e.getBoundingClientRect();return {center:Math.abs(r.x+r.width/2-innerWidth/2),top:r.top,height:innerHeight}")
        require(geometry['center'] < 30 and geometry['top'] < geometry['height'] * .35, 'toast is not upper centered')
        app.wait(lambda: not app.find('[data-testid=connection-toast]'), 6)
        action(screen_a['id'], 'test').click()
        toast('連線測試成功')
        require(screens[0].events, 'Screen test did not reach real MCP fixture')
        result['checks']['nativeHostAndScreenTestsWithTransientToast'] = True

        stage = 'alias-bounds'
        print('STAGE ' + stage, flush=True)
        action(computer['id'], 'edit').click()
        require(app.node('credential').get_attribute('value') == '', 'editing read Token back')
        app.fill('display-name', '漢' * 21)
        app.click('儲存', '[data-testid=runtime-host-form] button')
        require('20' in app.node('form-error').text, '21-character alias accepted')
        app.fill('display-name', '漢' * 20)
        app.click('儲存', '[data-testid=runtime-host-form] button')
        toast('儲存成功')
        require(app.profiles()[0]['displayName'] == '漢' * 20, '20 Chinese characters were not saved')
        action(computer['id'], 'edit').click()
        app.fill('display-name', '')
        app.click('儲存', '[data-testid=runtime-host-form] button')
        toast('儲存成功')
        require('displayName' not in app.profiles()[0], 'blank alias did not restore default')
        action(computer['id'], 'edit').click()
        app.fill('display-name', '開發助手')
        app.click('儲存', '[data-testid=runtime-host-form] button')
        toast('儲存成功')
        result['checks']['unicodeBoundaryAndDefault'] = True

        stage = 'grouped-drag'
        print('STAGE ' + stage, flush=True)
        baseline = {value['id']: value['revision'] for value in app.profiles() + app.screens()}
        drag(agent_b['id'], agent_a['id'])
        app.wait(lambda: order('agent') == [agent_b['id'], agent_a['id']])
        drag(screen_b['id'], screen_a['id'])
        app.wait(lambda: order('screen') == [screen_b['id'], screen_a['id']])
        before = config.read_bytes()
        drag(agent_b['id'], computer['id'])
        time.sleep(.3)
        require(config.read_bytes() == before, 'cross-type drag changed saved configuration')
        require(all(value['revision'] == baseline[value['id']] for value in app.profiles() + app.screens()), 'ordering changed record revisions')
        require(app.driver.execute_script('return window.__connectionPointers.length>=3 && window.__connectionPointers.every(e=>e.trusted)'), 'sorting did not use trusted mouse input')
        result['checks']['trustedSameTypeDragAndCrossTypeRefusal'] = True

        stage = 'restart-and-navigation'
        print('STAGE ' + stage, flush=True)
        app.close()
        app.launch(config)
        require([value['id'] for value in app.profiles() if value['category'] == 'agent'] == [agent_b['id'], agent_a['id']], 'agent sort not persisted')
        require([value['id'] for value in app.screens()] == [screen_b['id'], screen_a['id']], 'Screen sort not persisted')
        require(app.profiles()[0]['agentTool'] == '開發助手', 'Chinese alias lost on restart')
        app.wait(lambda: app.find('[data-testid="runtime-' + agent_b['id'] + '"]'))
        navigation = app.driver.execute_script("return [...document.querySelectorAll('.desktop-sidebar [data-testid^=runtime-]')].map(e=>e.dataset.testid)")
        require(navigation.index('runtime-' + agent_b['id']) < navigation.index('runtime-' + agent_a['id']), 'navigation does not follow saved Agent order')
        result['checks']['restartAndNavigation'] = True

        stage = 'manual-terminal'
        print('STAGE ' + stage, flush=True)
        name = create_shell(app, fixture, app.profiles()[0])
        require(not app.find('[data-testid=launch-coding-cli]'), 'launch CLI button remains')
        app.terminal('codex --help\r')
        app.wait(lambda: 'codex --help\r' in fixture.snapshot()['sessions'][name]['input'])
        result['checks']['manualTerminalWithoutLaunchControl'] = True
        result['webview'] = app.metadata
        result['passed'] = True
    except Exception as error:
        result['webview'] = app.metadata if app else {}
        result['failure'] = {'stage': stage, 'type': type(error).__name__}
        if type(error) is RuntimeError:
            result['failure']['code'] = str(error)
        if app and app.driver:
            try:
                if not app.find('[data-testid=credential]') or not app.node('credential').get_attribute('value'):
                    app.driver.save_screenshot(str(root / 'failure.png'))
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
        for upstream in screens:
            upstream.close()
        if fixture:
            fixture.close()
        (root / 'result.json').write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps({'passed': result['passed'], 'checks': result['checks'], 'failure': result.get('failure')}), flush=True)
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())