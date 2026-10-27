"""Real Codex approval and reconnect cases in an owned official Runtime."""
import argparse
from contextlib import ExitStack
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import secrets
import shlex
import shutil
import subprocess
import sys
import tempfile
import time


ROOT = Path(__file__).resolve().parents[2]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


suite = load('codex_official_tui', ROOT / 'scripts/tui-regression/official_runtime.py')
official, bridge, driver = suite.dependencies()


class Gateway(official.TlsRuntimeGateway):
    def __init__(self, root, harness):
        self.attaches = []
        super().__init__(root, harness)

    async def pty(self, request):
        self.attaches.append({'name': request.match_info['name'],
                             'since': request.query.get('since'),
                             'tokenHash': hashlib.sha256(request.headers.get('Authorization', '').encode()).hexdigest()})
        return await super().pty(request)

    def disconnect(self, binding):
        async def abort():
            matches = [ws for ws in self.live_ws if ws._req and ws._req.match_info['name'] == binding]
            assert len(matches) == 1, 'Expected one owned work WSS'
            matches[0]._req.transport.abort()
        self.call(abort())


def decoded(value):
    yield value
    if isinstance(value, dict):
        for item in value.values():
            yield from decoded(item)
    elif isinstance(value, list):
        for item in value:
            yield from decoded(item)
    elif isinstance(value, str):
        try:
            nested = json.loads(value)
        except (ValueError, TypeError):
            return
        if nested != value:
            yield from decoded(nested)


class Runtime:
    def __init__(self, args, root):
        self.cleanup = ExitStack()
        try:
            self.setup(args, root)
        except BaseException:
            self.cleanup.close()
            raise

    def setup(self, args, root):
        self.args, self.root = args, root
        self.bindings = set()
        self.harness = official.linux.Harness(root, args.runtime_binary)
        h = self.harness
        self.cleanup.callback(self.remove_broker_directories)
        self.cleanup.callback(h.close)
        tools = h.home / '.local/share/art-term/tools'
        tools.mkdir(parents=True, mode=0o700)
        for name in ('instance_bridge.py', 'instance-shell.py', 'instance-shell.bash'):
            source = args.bridge_source if name == 'instance_bridge.py' else ROOT / 'scripts' / name
            shutil.copy2(source, tools / name)
        shutil.copy2(ROOT / 'scripts/openab-runtime/tmux-v2/tmux-shell.py', tools / 'tmux-shell.py')
        tmux = h.helper / 'tmux-v2'
        tmux.mkdir()
        shutil.copy2(args.tmux, tmux / 'tmux')
        shutil.copy2(args.history_bundle, tmux / 'tmux-history.cjs')
        shutil.copy2(args.node, h.helper / 'node')
        bindir = h.home / 'bin'
        bindir.mkdir()
        (bindir / 'codex').symlink_to(args.codex)
        self.config = h.home / '.codex'
        self.config.mkdir(mode=0o700)
        shutil.copyfile(args.auth_file, self.config / 'auth.json')
        (self.config / 'auth.json').chmod(0o600)
        self.workspace = root / 'workspace'
        self.workspace.mkdir()
        (self.config / 'config.toml').write_text(
            'model=' + json.dumps(args.model) + '\nmodel_reasoning_effort="medium"\n'
            '[projects.' + json.dumps(str(self.workspace)) + ']\ntrust_level="trusted"\n')
        (h.home / '.bashrc').write_text(
            'export PATH=' + shlex.quote(str(bindir)) + ':$PATH\n'
            'export CODEX_HOME=' + shlex.quote(str(self.config)) + '\n'
            'cd ' + shlex.quote(str(self.workspace)) + '\n'
            'export PS1="ARTTERM_TUI_READY> "\n')
        proxy = root / 'proxy'
        proxy.mkdir()
        self.gateway = Gateway(proxy, h)
        self.cleanup.callback(self.gateway.close)
        self.services = []
        for name in ('A', 'B'):
            service = official.McpFixture(name, sse=name == 'B')
            self.services.append(service)
            self.cleanup.callback(service.close)

    def close(self):
        try:
            h = self.harness
            h.owned.update(item['name'] for item in h.api('GET', '/admin/sessions')[1]['sessions'])
            self.bindings.update(name for name in h.owned if name.startswith('th2-'))
        finally:
            self.cleanup.close()

    def remove_broker_directories(self):
        for binding in self.bindings:
            path = Path('/tmp') / ('art-term-' + str(os.getuid())) / hashlib.sha256(binding.encode()).hexdigest()[:24]
            deadline = time.monotonic() + 5
            while path.exists():
                try:
                    path.rmdir()
                except OSError:
                    if time.monotonic() >= deadline:
                        raise
                    time.sleep(.05)


class TuiSurface:
    def __init__(self, runtime):
        self.runtime = runtime
        self.home = runtime.root / 'client'
        self.home.mkdir(mode=0o700)
        self.app = None
        try:
            self.setup()
        except BaseException:
            self.close()
            raise

    def setup(self):
        runtime = self.runtime
        self.start()
        app = self.app
        suite.action(app, 'New Host')
        app.wait_text('HTTPS Runtime URL')
        app.send('Official Runtime\t' + runtime.gateway.origin + '\t' + runtime.harness.credential)
        app.send(b'\x13')
        app.wait(lambda: 'HTTPS Runtime URL' not in app.text(), 'saved Host')
        app.idle()
        for service in runtime.services:
            suite.action(app, 'New Screen')
            app.wait_text('新增 Screen')
            app.send('Screen ' + service.name + '\t' + service.url + '\t' + service.token)
            app.send(b'\x13')
            app.wait(lambda: '新增 Screen' not in app.text(), 'saved Screen')
            app.idle()
        suite.click(app, '+ New Shell')
        app.wait_text('ARTTERM_TUI_READY>')
        self.binding = next(item['name'] for item in runtime.harness.api('GET', '/admin/sessions')[1]['sessions']
                            if item['name'].startswith('th2-'))
        app.idle()

    def start(self):
        self.app = driver.Terminal(self.runtime.args.binary, self.home,
                                   self.runtime.gateway.ca, rows=50, columns=180)
        self.app.start()

    def select(self, name):
        suite.choose(self.app, 'Screen ' + name)
        self.app.wait_text('Tools ready')
        self.app.prefix('t')

    def clear(self):
        suite.click(self.app, '[Desktop]')
        suite.click(self.app, 'Clear tools')
        suite.click(self.app, '[Yes]')
        self.app.wait_text('Desktop: 未設定')
        self.app.prefix('t')

    def reopen(self):
        self.app.prefix('q')
        self.app.finish()
        self.app.close()
        self.start()
        suite.click(self.app, self.binding)
        suite.click(self.app, '[Yes]')
        self.app.wait_text('Tools ready')
        self.app.prefix('t')

    def send(self, value):
        self.app.prefix('t')
        self.app.send(value)

    def submit(self, value):
        self.app.settle(.3)
        self.app.wait_text('Live |')
        self.send('\x1b[200~' + value + '\x1b[201~')
        self.app.settle(.3)
        self.app.send('\r')

    def pump(self):
        self.app.pump()

    def text(self):
        return self.app.text()

    def save(self, output, name):
        self.app.save(output, name)

    def close(self):
        if self.app:
            self.app.close()


class Codex:
    def __init__(self, runtime, surface, case, output):
        self.runtime, self.surface, self.case, self.output = runtime, surface, case, output
        self.rollout = None
        self.offset = 0
        self.approvals = []
        self.approval_visible = False
        self.completed_turns = set()

    def events(self):
        files = list((self.runtime.config / 'sessions').rglob('*.jsonl'))
        if files:
            self.rollout = max(files, key=lambda p: p.stat().st_mtime_ns)
        if self.rollout is None:
            return []
        events = []
        for line in self.rollout.read_text().splitlines():
            try:
                events.append(json.loads(line))
            except ValueError:
                pass
        return events

    def identity(self):
        binding = self.surface.binding.encode()
        found = []
        for path in Path('/proc').glob('[0-9]*'):
            try:
                argv = (path / 'cmdline').read_bytes().split(b'\0')
                if argv[0] == b'codex' and any(b'"' + binding + b'"' in arg for arg in argv):
                    fields = (path / 'stat').read_text().rsplit(')', 1)[1].split()
                    if fields[0] != 'Z':
                        found.append({'pid': int(path.name), 'start': fields[19]})
            except (OSError, IndexError):
                pass
        assert len(found) == 1, 'Expected one owned Codex process for this Shell'
        return found[0]

    def start(self, prompt, extra=None, approve=False, during=None):
        number = int(self.case[1:])
        modes = {2: 'read-only', 3: 'workspace-write', 4: 'danger-full-access',
                 5: 'read-only', 6: 'workspace-write', 7: 'danger-full-access'}
        command = ['codex', '--no-daemon', '--no-alt-screen']
        if number in (13, 14):
            command += ['-s', 'read-only', '-a', 'never']
        elif number in modes:
            command += ['-s', modes[number], '-a', 'never' if number < 5 else 'on-request']
        else:
            command += ['--yolo']
        command += extra or []
        command += [prompt]
        self.surface.submit(shlex.join(command))
        return self.finish_turn(approve, during)

    def finish_turn(self, approve=False, during=None):
        deadline = time.monotonic() + self.runtime.args.timeout
        while time.monotonic() < deadline:
            self.surface.pump()
            if during:
                during()
            events = self.events()
            recent = events[self.offset:]
            complete = next((item for item in recent if item.get('type') == 'event_msg'
                             and item['payload'].get('type') == 'task_complete'
                             and item['payload']['turn_id'] not in self.completed_turns), None)
            if complete:
                turn_id = complete['payload']['turn_id']
                start = next(i for i, item in enumerate(events) if item.get('type') == 'event_msg'
                             and item['payload'].get('type') == 'task_started'
                             and item['payload']['turn_id'] == turn_id)
                self.completed_turns.add(turn_id)
                self.offset = len(events)
                return events[start:]
            text = self.surface.text()
            compact = ''.join(text.split())
            prompt_visible = 'Allowtheart_termMCPservertoruntool"call_target_tool"?' in compact and 'name:browser_title' in compact
            if prompt_visible and not self.approval_visible:
                assert approve, 'Unexpected approval request'
                self.surface.save(self.output, 'approval-' + str(len(self.approvals)))
                self.approvals.append('denied browser_title' if approve == 'deny' else 'allowed browser_title')
                self.surface.send('\x1b' if approve == 'deny' else '\r')
                deadline = min(deadline, time.monotonic() + 90)
            self.approval_visible = prompt_visible
        raise AssertionError('Actual Codex turn did not finish before the deadline')

    def next_turn(self, prompt, approve=False):
        self.surface.submit(prompt)
        return self.finish_turn(approve)

    def session_id(self):
        return next(item['payload']['id'] for item in self.events() if item['type'] == 'session_meta')

    def resume(self, prompt, yolo=False):
        old = self.identity()
        session = self.session_id()
        self.surface.submit('/exit')
        deadline = time.monotonic() + 15
        while Path('/proc', str(old['pid'])).exists() and time.monotonic() < deadline:
            self.surface.pump()
        assert not Path('/proc', str(old['pid'])).exists(), 'Original Codex did not exit'
        self.rollout = None
        self.offset = 0
        flags = ['--yolo'] if yolo else ['-s', 'read-only', '-a', 'never']
        command = ['codex', '--no-daemon', '--no-alt-screen', *flags, 'resume', session, prompt]
        self.surface.submit(shlex.join(command))
        events = self.finish_turn()
        assert self.identity() != old, 'Resume reused the old Codex process'
        assert self.session_id() == session, 'Resume created another conversation'
        return events, old

    def save(self):
        events = self.events()
        (self.output / 'codex-events.json').write_text(json.dumps(events, indent=2) + '\n')
        try:
            process = self.identity()
        except AssertionError as error:
            process = {'unavailable': str(error)}
        (self.output / 'process.json').write_text(json.dumps(process) + '\n')
        (self.output / 'fixture-events.json').write_text(json.dumps(
            {service.name: service.events for service in self.runtime.services}, indent=2) + '\n')
        self.surface.save(self.output, 'terminal')


PROMPT = (
    'Use only the art_term MCP tools. Read current_target and list_target_tools. '
    'Then call_target_tool with browser_title, empty arguments, and the exact current target. '
    'Do not run shell commands or alternative tools. If policy refuses, stop and quote the error. '
    'Report only the actual tool result. End with ISSUE8_DONE.'
)


def tool_values(events):
    for item in events:
        payload = item.get('payload', {})
        if item.get('type') == 'response_item' and payload.get('type') in (
                'custom_tool_call_output', 'function_call_output', 'mcp_tool_call_output'):
            yield from decoded(payload)


def browser_calls(runtime):
    return [e for service in runtime.services for e in service.events
            if e['method'] == 'tools/call' and e['params']['name'] == 'browser_title']


def check_turn(events, runtime, expect_call, desktop='A', previous_calls=0):
    values = list(tool_values(events))
    assert any(isinstance(v, dict) and isinstance(v.get('target'), dict) for v in values), \
        'Read-only current_target was blocked before desktop dispatch'
    assert any(isinstance(v, dict) and isinstance(v.get('tools'), list) for v in values), \
        'Read-only list_target_tools was blocked before desktop dispatch'
    calls = browser_calls(runtime)[previous_calls:]
    errors = [v for v in values if isinstance(v, dict) and v.get('isError')]
    if expect_call:
        assert len(calls) == 1, 'Expected exactly one actual browser_title request'
        assert any(isinstance(v, dict) and v.get('desktop') == desktop and v.get('page') == 'about:blank'
                   for v in values), 'Actual browser_title result was not returned to Codex'
        assert not errors, 'Codex received an unexpected MCP tool error'
        return 'SUCCESS'
    assert not calls, 'Policy refusal still dispatched a desktop operation'
    assert any(v == 'MCP tool call requires approval, but approval policy is never' for v in values), \
        'Expected a precise policy refusal for the side-effect-capable dispatcher'
    return 'CORRECT_REFUSAL'


def unready(events, runtime, previous_calls):
    assert len(browser_calls(runtime)) == previous_calls, 'Unavailable target still dispatched an operation'
    assert any(isinstance(v, str) and v.startswith('No ready target, stale expected_target, or instance-mcp request failed.')
               for v in tool_values(events)), 'Codex did not receive the actual broker refusal'


def target_from(events):
    return next(v['target'] for v in tool_values(events)
                if isinstance(v, dict) and isinstance(v.get('target'), dict))


def run_case(runtime, surface, codex, result):
    number = int(codex.case[1:])
    if number == 8:
        events = codex.start('Read current_target and list_target_tools. Do not call a desktop operation. Quote actual errors. End with ISSUE8_DONE.')
        unready(events, runtime, 0)
        old = codex.identity()
        surface.select('A')
        result['outcome'] = check_turn(codex.next_turn(PROMPT), runtime, True)
        assert codex.identity() == old, 'Dynamic selection restarted Codex'
        return
    surface.select('A')
    if number == 11:
        reconnect_during_tool(runtime, surface, codex, result)
        return
    events = codex.start(PROMPT, approve=number in (5, 6, 7))
    result['outcome'] = check_turn(events, runtime, number not in (2, 3, 13, 14))
    context = next(item['payload'] for item in events if item['type'] == 'turn_context')
    result['policy'] = {key: context[key] for key in ('approval_policy', 'sandbox_policy')}
    initial_target = target_from(events)
    initial_identity = codex.identity()
    result['initialProcess'] = initial_identity
    if number == 5:
        assert codex.approvals == ['allowed browser_title'], 'On-request read-only did not show a handled approval'
        events = codex.next_turn(PROMPT, approve='deny')
        assert len(browser_calls(runtime)) == 1, 'User rejection still dispatched a desktop tool'
        assert any(isinstance(v, str) and any(word in v.lower() for word in ('denied', 'rejected', 'cancelled', 'canceled'))
                   for v in tool_values(events)), 'User rejection was not returned to Codex'
    elif number == 9:
        surface.select('B')
        prompt = ('Read current_target and list_target_tools. For this isolated regression test, call browser_title '
                  'once using the stale expected_target ' + json.dumps(initial_target) + '. It must reject. '
                  'Then call browser_title once with the new exact target. Do not run shell commands. End with ISSUE8_DONE.')
        events = codex.next_turn(prompt)
        values = list(tool_values(events))
        assert any(isinstance(v, dict) and v.get('desktop') == 'B' for v in values), 'New selection did not route to B'
        assert any(isinstance(v, str) and v.startswith('No ready target, stale expected_target') for v in values), 'Stale selection did not reject'
        assert len(browser_calls(runtime)) == 2, 'Stale selection reached an upstream desktop'
    elif number == 10:
        surface.clear()
        events = codex.next_turn(PROMPT)
        unready(events, runtime, 1)
        surface.select('B')
        runtime.services[1].token = secrets.token_hex(32)
        events = codex.next_turn('Read current_target and list_target_tools. Stop at an actual tool failure and quote it. Do not run shell commands. End with ISSUE8_DONE.')
        unready(events, runtime, 1)
    elif number == 12:
        surface.reopen()
        events = codex.next_turn(PROMPT)
        check_turn(events, runtime, True, previous_calls=1)
        assert target_from(events) == initial_target, 'App reopen reconfigured the broker'
    elif number in (13, 14):
        events, old = codex.resume(PROMPT, yolo=number == 14)
        result['outcome'] = check_turn(events, runtime, number == 14)
        result['previousProcess'] = old
        context = next(item['payload'] for item in events if item['type'] == 'turn_context')
        result['resumedPolicy'] = {key: context[key] for key in ('approval_policy', 'sandbox_policy')}
    if number not in (13, 14):
        assert codex.identity() == initial_identity, 'Scenario changed the Codex process'
    result['approvals'] = codex.approvals


def reconnect_during_tool(runtime, surface, codex, result):
    service = runtime.services[0]
    service.pause, service.pause_tool = True, 'browser_title'
    original = {}

    def disconnect():
        if original or not service.started.is_set():
            return
        original.update(process=codex.identity(), session=codex.session_id())
        before = [e for e in runtime.gateway.attaches if e['name'] == surface.binding]
        h = runtime.harness
        generation = next(v['generation'] for v in h.api('GET', '/admin/sessions')[1]['sessions'] if v['name'] == surface.binding)
        runtime.gateway.disconnect(surface.binding)
        try:
            deadline = time.monotonic() + 4
            while sum(e['name'] == surface.binding for e in runtime.gateway.attaches) <= len(before) and time.monotonic() < deadline:
                surface.pump()
            after = [e for e in runtime.gateway.attaches if e['name'] == surface.binding]
            assert len(after) == len(before) + 1, 'Work WSS did not reconnect exactly once'
            assert after[-1]['tokenHash'] == before[-1]['tokenHash'], 'Reconnect changed the work token'
            assert int(after[-1]['since']) > 0, 'Reconnect lost its output cursor'
            assert next(v['generation'] for v in h.api('GET', '/admin/sessions')[1]['sessions'] if v['name'] == surface.binding) == generation
            result['reconnect'] = {'before': before[-1], 'after': after[-1]}
        finally:
            service.release.set()

    try:
        events = codex.start(PROMPT, during=disconnect)
        result['outcome'] = check_turn(events, runtime, True)
        assert original, 'WSS was not interrupted during an actual tool request'
        assert codex.identity() == original['process'], 'Reconnect restarted Codex'
        assert codex.session_id() == original['session'], 'Reconnect changed the conversation'
        deadline = time.monotonic() + 10
        while 'about:blank' not in surface.text() and time.monotonic() < deadline:
            surface.pump()
        assert 'about:blank' in surface.text(), 'Resumed App did not render the delayed actual tool result'
        result['inputFenceVisible'] = '輸入結果不確定' in surface.text()
    finally:
        service.release.set()


def arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('binary', 'runtime-binary', 'history-bundle', 'node', 'tmux', 'codex', 'auth-file'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--bridge-source', type=Path, default=ROOT / 'scripts/instance_bridge.py',
                        help='Remote helper source for an explicit baseline comparison')
    parser.add_argument('--model', default='gpt-6.1-sol')
    parser.add_argument('--case', choices=['C%02d' % n for n in range(1, 15)], required=True)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--timeout', type=int, default=150)
    args = parser.parse_args()
    for name in ('binary', 'runtime_binary', 'history_bundle', 'node', 'tmux', 'codex', 'auth_file', 'bridge_source'):
        setattr(args, name, getattr(args, name).resolve(strict=True))
    return args


def main():
    args = arguments()
    args.output_dir.mkdir(mode=0o700, parents=True, exist_ok=False)
    result = {'case': 'L-' + args.case, 'passed': False, 'platform': 'Linux TUI',
              'codexVersion': subprocess.check_output([str(args.codex), '--version'], text=True).strip(),
              'source': subprocess.check_output(['git', '-C', str(ROOT), 'rev-parse', 'HEAD'], text=True).strip(),
              'bridgeSha256': hashlib.sha256(args.bridge_source.read_bytes()).hexdigest(),
              'harnessSha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'binarySha256': hashlib.sha256(args.binary.read_bytes()).hexdigest()}
    runtime = surface = codex = None
    try:
        with tempfile.TemporaryDirectory(prefix='art-term-') as directory:
            try:
                runtime = Runtime(args, Path(directory))
                surface = TuiSurface(runtime)
                runtime.bindings.add(surface.binding)
                result['binding'] = surface.binding
                codex = Codex(runtime, surface, args.case, args.output_dir)
                run_case(runtime, surface, codex, result)
                result['process'] = codex.identity()
                result['sessionId'] = codex.session_id()
                result['passed'] = True
            finally:
                try:
                    if codex:
                        codex.save()
                finally:
                    try:
                        if surface:
                            surface.close()
                    finally:
                        if runtime:
                            runtime.close()
    except Exception as error:
        result['passed'] = False
        result['error'] = str(error)
    (args.output_dir / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    summary = dict(result)
    if summary.get('error'):
        summary['error'] = summary['error'].splitlines()[0]
    print(json.dumps(summary), flush=True)
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
