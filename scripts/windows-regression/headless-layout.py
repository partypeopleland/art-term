"""Background browser UI checks only. No native App, IPC, DPAPI or real Host claims."""
import argparse
import json
import tempfile
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
    temp = Path(tempfile.gettempdir()).resolve()
    if not any(p.name.startswith('art-term-') and p.parent == temp for p in [output, *output.parents]):
        raise ValueError('Output must stay inside a task-owned art-term-* temporary directory')
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
        host = next(e for e in driver.find_elements(By.CSS_SELECTOR, '.pane .xterm-screen') if e.is_displayed())
        ActionChains(driver).scroll_from_origin(ScrollOrigin.from_element(host), 0, amount).perform()

    def terminal_text():
        return js("return document.querySelector('.xterm-rows')?.textContent || ''")

    try:
        driver.get(args.url)
        node('runtime-demo-host')
        check(abs(width() - 360) < 1, 'default sidebar is not 360px')
        driver.set_window_size(1820, 1000)
        wait.until(lambda _: abs(width() - 200) < 1)
        checks['fixed360AfterWindowExpansion'] = True
        drag('sidebar-divider', 160, 0)
        wait.until(lambda _: abs(width() - 520) < 2)
        driver.set_window_size(1360, 900)
        wait.until(lambda _: abs(width() - 360) < 2)
        checks['dragWidthRetainedOnResize'] = True
        preview_before = node('preview-divider').rect['y']
        drag('preview-divider', 0, -70)
        wait.until(lambda _: abs(node('preview-divider').rect['y'] - preview_before + 70) < 2)
        checks['dragPreviewHeight'] = True
        node('runtime-demo-host').find_element(By.CSS_SELECTOR, '.new-connection').click()
        wait.until(lambda _: 'ROW-1100' in terminal_text())
        js("window.__inlineXterm=document.querySelector('.pane .xterm')")
        def visible_rows():
            return js("const host=document.querySelector('.pane .xterm-screen'),box=host.getBoundingClientRect();return [...host.querySelectorAll('.xterm-rows>div')].filter(e=>{const b=e.getBoundingClientRect();return b.bottom>box.top&&b.top<box.bottom}).map(e=>e.innerText).filter(Boolean)")
        def same_renderer():
            check(js("return window.__inlineXterm===document.querySelector('.pane .xterm')"), 'xterm instance/DOM was replaced')
            check(not driver.find_elements(By.CSS_SELECTOR, '.desktop-history,.history-viewport,[data-testid=history-close]'), 'second reader/return control exists')
        live = visible_rows()
        original_font = js("return getComputedStyle(document.querySelector('.xterm-rows')).font")
        wheel(-34)
        wait.until(lambda _: visible_rows() != live)
        same_renderer()
        first = js("return {request:window.__desktopPreview.requests.find(r=>r.op===\u0027scroll\u0027&&r.lines<0),cellHeight:document.querySelector(\u0027.xterm-rows>div\u0027).getBoundingClientRect().height}")
        check(first["request"] and first["request"]["lines"] <= -1, "first34px wheel did not reach an earlier row")
        ids = [int(row.split("ROW-")[1].split()[0]) for row in visible_rows() if "ROW-" in row]
        live_ids = [int(row.split("ROW-")[1].split()[0]) for row in live if "ROW-" in row]
        check(ids[0] < live_ids[0] and all(b == a + 1 for a, b in zip(ids, ids[1:])), "first wheel does not show earlier consecutive rows")
        checks["firstWheelMeasurement"] = first
        check(js("return !window.__desktopPreview.requests.some(r=>['viewport','older','ack','keep'].includes(r.op))"), 'Windows still polls snapshots')
        checks['first34pxSameOriginalXterm'] = True
        for _ in range(6): wheel(-5000)
        wait.until(lambda _: any('ROW-0001 ' in row for row in visible_rows()))
        same_renderer()
        check(any('桌面終端示範' in row for row in visible_rows()), 'Unicode history is not actually visible')
        held = visible_rows()
        for _ in range(3):
            js('window.__desktopPreview.append(7)')
            driver.execute_async_script('const done=arguments[0];setTimeout(done,80)')
            check(visible_rows() == held, 'continuous output moved held original viewport')
        check(js("return getComputedStyle(document.querySelector('.xterm-rows')).font") == original_font, 'history swaps original font')
        check(js("return [...document.querySelectorAll('.xterm-rows span')].some(e=>getComputedStyle(e).color==='rgb(86, 180, 233)')"), 'history lost original RGB style')
        row = next(e for e in driver.find_elements(By.CSS_SELECTOR, ".xterm-rows>div") if "ROW-0001 " in e.text)
        ActionChains(driver).move_to_element_with_offset(row, -row.rect["width"] / 2 + 8, 0).click_and_hold().move_by_offset(200, 0).release().perform()
        check(js("return document.querySelectorAll(\u0027.xterm-selection div\u0027).length>0"), "trusted mouse selection missing from original xterm")
        for _ in range(6): wheel(5000)
        wait.until(lambda _: any('ROW-1121 ' in row for row in visible_rows()))
        js('window.__desktopPreview.append(1)')
        wait.until(lambda _: any('ROW-1122 ' in row for row in visible_rows()))
        same_renderer()
        checks['thousandRowsHeldOutputAndNaturalFollow'] = True
        # A real theme change reaches the same renderer, not just an outer class.
        before_color = js("return getComputedStyle(document.querySelector('.xterm-rows')).color")
        node('desktop-settings').click()
        next(e for e in driver.find_elements(By.CSS_SELECTOR,'.theme-option') if '柔光' in e.text).click()
        node('desktop-workspace').click()
        wait.until(lambda _: js("return getComputedStyle(document.querySelector('.xterm-rows')).color") != before_color)
        same_renderer()
        check(js("return getComputedStyle(document.querySelector('.xterm-rows')).font") == original_font, 'theme changed font when entering history')
        # CSS font styling remains on the original row DOM throughout control.
        js("const s=document.createElement('style');s.id='owned-font-proof';s.textContent='.xterm-rows{font-family:monospace!important}';document.head.append(s)")
        check('monospace' in js("return getComputedStyle(document.querySelector('.xterm-rows')).fontFamily"), 'computed font style did not actually change')
        same_renderer()
        js("document.getElementById('owned-font-proof').remove()")
        checks['originalRgbSelectionFontAndActualThemeChange'] = True
        driver.save_screenshot(str(output / 'workspace-inline-original-xterm.png'))
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
        check(not driver.find_elements(By.CSS_SELECTOR, '[data-testid=runtime-host-form]'), 'add form is visible before add is clicked')
        node('runtime-create').click()
        check(len(driver.find_elements(By.CSS_SELECTOR, 'input[type=radio][data-testid^=connection-type-]')) == 3, 'connection type radios missing')
        node('credential').send_keys('fixture-draft')
        check(not driver.find_elements(By.CSS_SELECTOR, '[role=dialog]'), 'connection management still a modal')
        check(driver.find_elements(By.CSS_SELECTOR, '[data-host-id] button'), 'existing connection actions lost')
        driver.save_screenshot(str(output / 'settings-connections.png'))
        check(not driver.find_elements(By.CSS_SELECTOR, '[data-testid=settings-screens]'), 'separate Screen settings remains')
        node('settings-appearance').click()
        check(not driver.find_elements(By.CSS_SELECTOR, '[data-testid=credential]'), 'secret form retained across categories')
        node('settings-connections').click()
        check(not driver.find_elements(By.CSS_SELECTOR, '[data-testid=runtime-host-form]'), 'form reopened across categories')
        node('runtime-create').click()
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
