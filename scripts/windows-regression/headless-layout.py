"""Background browser UI checks only. No native App, IPC, DPAPI or real Host claims."""
import argparse
import json
import subprocess
from pathlib import Path
from selenium import webdriver
from selenium.webdriver.edge.service import Service
from selenium.webdriver.common.by import By
from selenium.webdriver.common.action_chains import ActionChains
from selenium.webdriver.common.actions.wheel_input import ScrollOrigin
from selenium.webdriver.support.ui import WebDriverWait


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--driver', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--url', default='http://127.0.0.1:1421/?desktop')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    output = Path(args.output).resolve()
    if not output.is_relative_to(root / 'target' / 'windows-regression'):
        raise ValueError('Output must stay in target/windows-regression')
    output.mkdir(parents=True, exist_ok=True)
    options = webdriver.EdgeOptions()
    options.add_argument('--headless=new')
    options.add_argument('--no-first-run')
    options.add_argument('--window-size=1280,900')
    service = Service(args.driver, popen_kw={'creation_flags': getattr(subprocess, 'CREATE_NO_WINDOW', 0)})
    driver = webdriver.Edge(service=service, options=options)
    wait = WebDriverWait(driver, 15)
    result = {'kind': 'headless-browser-fixture', 'nativeE2E': 'NOT_RUN', 'passed': False, 'checks': {}}
    checks = result['checks']

    def js(script, *values):
        return driver.execute_script(script, *values)

    def node(testid):
        return wait.until(lambda _: next((e for e in driver.find_elements(By.CSS_SELECTOR, '[data-testid="' + testid + '"]') if e.is_displayed()), None))

    def check(value, message):
        if not value:
            raise AssertionError(message)

    def width():
        return js("return document.querySelector('.desktop-sidebar').getBoundingClientRect().width")

    def drag(testid, dx, dy):
        ActionChains(driver).move_to_element(node(testid)).click_and_hold().move_by_offset(dx, dy).release().perform()

    def wheel(amount):
        host = driver.find_element(By.CSS_SELECTOR, '.history-viewport')
        ActionChains(driver).scroll_from_origin(ScrollOrigin.from_element(host), 0, amount).perform()

    def first():
        return js("return Number(document.querySelector('.history-line')?.dataset.lineId)")

    def anchor():
        return js("const h=document.querySelector('.history-viewport');const e=[...h.querySelectorAll('.history-line')].find(e=>e.offsetTop+e.offsetHeight>h.scrollTop+4);return {id:e.dataset.lineId,offset:e.getBoundingClientRect().top-h.getBoundingClientRect().top};")

    try:
        driver.get(args.url)
        node('runtime-demo-host')
        check(abs(width() - 200) < 1, 'default sidebar is not 200px')
        driver.set_window_size(1820, 1000)
        wait.until(lambda _: abs(width() - 200) < 1)
        checks['fixed200AfterWindowExpansion'] = True
        drag('sidebar-divider', 160, 0)
        wait.until(lambda _: abs(width() - 360) < 2)
        driver.set_window_size(1360, 900)
        wait.until(lambda _: abs(width() - 360) < 2)
        checks['dragWidthRetainedOnResize'] = True
        preview_before = node('preview-divider').rect['y']
        drag('preview-divider', 0, -70)
        wait.until(lambda _: abs(node('preview-divider').rect['y'] - preview_before + 70) < 2)
        checks['dragPreviewHeight'] = True
        node('runtime-demo-host').find_element(By.CSS_SELECTOR, '.new-connection').click()
        wait.until(lambda _: js("return !!document.querySelector('.history-line')"))
        wait.until(lambda _: js("return window.__desktopPreview.requests.filter(r=>r.op==='older').length>=3"))
        checks['threePagesPrefetchedBeforeWheel'] = True
        check(js("const h=document.querySelector('.history-viewport');return h.scrollWidth<=h.clientWidth && getComputedStyle(h).overflowX==='hidden' && [...h.querySelectorAll('.history-line')].some(e=>e.offsetHeight>17);"), 'long lines do not wrap without horizontal overflow')
        checks['longLinesWrapWithoutHorizontalOverflow'] = True
        original = first()
        wheel(-10000)
        wait.until(lambda _: first() < original)
        wait.until(lambda _: js("const h=document.querySelector('.history-viewport');return h.scrollTop>100"))
        held = anchor()
        driver.set_window_size(1000, 900)
        wait.until(lambda _: anchor()['id'] == held['id'] and abs(anchor()['offset'] - held['offset']) <= 2)
        check(js("const h=document.querySelector('.history-viewport');return h.scrollWidth<=h.clientWidth"), 'resize creates horizontal overflow')
        checks['wrappedAnchorOnResize'] = held
        for _ in range(4):
            before = first()
            wheel(-100000)
            wait.until(lambda _: first() < before)
        check(js("return document.querySelectorAll('.history-line').length") == 1024, 'history window is not bounded')
        checks['bounded1024'] = True
        held = anchor()
        js('window.__desktopPreview.append()')
        driver.execute_async_script('const done=arguments[0];setTimeout(done,750)')
        check(anchor()['id'] == held['id'] and abs(anchor()['offset'] - held['offset']) <= 2, 'new output moves held reading position')
        checks['heldDuringNewOutput'] = True
        driver.save_screenshot(str(output / 'workspace-wrapped.png'))
        node('desktop-toggle').click()
        node('choose-screen-demo-screen').click()
        check(not driver.find_elements(By.CSS_SELECTOR, '[data-testid=screen-picker]'), 'Screen picker waits for background selection')
        check(driver.find_element(By.CSS_SELECTOR, '.preview-loading').is_displayed(), 'selection loading is missing')
        check(driver.find_element(By.CSS_SELECTOR, '.pane').is_displayed(), 'CLI hidden while selecting Screen')
        wait.until(lambda _: js("const i=document.querySelector('img.shot');return i && i.complete && i.naturalWidth>0"))
        checks['nonblockingScreenLoading'] = True
        drag('sidebar-divider', 220, 0)
        drag('preview-divider', 0, 100)
        check(js("const w=document.querySelector('.shot-wrap'),i=document.querySelector('img.shot'),a=w.getBoundingClientRect(),b=i.getBoundingClientRect();return getComputedStyle(w).backgroundColor==='rgb(0, 0, 0)' && getComputedStyle(i).objectFit==='contain' && b.top>=a.top && b.bottom<=a.bottom+1 && b.width>b.height;"), 'preview does not contain image inside a black height-limited area')
        driver.save_screenshot(str(output / 'workspace-preview.png'))
        checks['previewContainBlackLetterbox'] = True
        node('desktop-settings').click()
        check(not driver.find_elements(By.CSS_SELECTOR, '[data-testid=desktop-manage-hosts]'), 'old host management button exists')
        check(not driver.find_elements(By.CSS_SELECTOR, '[data-testid=screen-settings], [data-testid=runtime-host-form]'), 'settings spreads all forms')
        driver.save_screenshot(str(output / 'settings-appearance.png'))
        node('settings-connections').click()
        node('credential').send_keys('fixture-draft')
        check(not driver.find_elements(By.CSS_SELECTOR, '[role=dialog]'), 'connection management still a modal')
        check(driver.find_elements(By.CSS_SELECTOR, '[data-host-id] button'), 'existing connection actions lost')
        driver.save_screenshot(str(output / 'settings-connections.png'))
        node('settings-screens').click()
        node('screen-settings')
        check(not driver.find_elements(By.CSS_SELECTOR, '[data-testid=credential]'), 'secret form retained across categories')
        node('settings-connections').click()
        check(node('credential').get_attribute('value') == '', 'secret draft restored after category switch')
        checks['categorizedSharedInlineSettingsAndDraftCleanup'] = True
        result['passed'] = True
    except Exception as error:
        result['error'] = str(error)
        driver.save_screenshot(str(output / 'failure.png'))
        raise
    finally:
        (output / 'result.json').write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding='utf-8')
        driver.quit()


if __name__ == '__main__':
    main()
