"""Fixed MCP clients exercise real stdio adapters and HTTP A/B routing."""
import importlib.util
import json
from pathlib import Path
import select
import subprocess
import threading
import sys
import time
import uuid
from fixture import Fixture

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT/'scripts/instance_bridge.py'
spec = importlib.util.spec_from_file_location('bridge', SCRIPT)
bridge = importlib.util.module_from_spec(spec); spec.loader.exec_module(bridge)


class Client:
    def __init__(self, binding=None, stdin=None, stdout=None):
        self.binding = binding or ('e2e-'+uuid.uuid4().hex[:20])
        self.p = subprocess.Popen([sys.executable, str(SCRIPT), 'stdio', '--binding', self.binding], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.n = 0
        self.call('initialize', {'protocolVersion': '2025-03-26', 'capabilities': {}, 'clientInfo': {'name': 'fixed-ai-client', 'version': '1'}})
    def call(self, method, params):
        self.n += 1
        self.p.stdin.write((json.dumps({'jsonrpc': '2.0', 'id': self.n, 'method': method, 'params': params})+'\n').encode()); self.p.stdin.flush()
        assert select.select([self.p.stdout], [], [], 25)[0], 'adapter timeout'
        r = json.loads(self.p.stdout.readline()); assert r['id'] == self.n
        return r['result']
    def tool(self, name, args=None): return self.call('tools/call', {'name': name, 'arguments': args or {}})
    def target(self): return json.loads(self.tool('current_target')['content'][0]['text'])['target']
    def execute(self, name, args=None, target=None): return self.tool('call_target_tool', {'name': name, 'arguments': args or {}, 'expected_target': target or self.target()})
    def close(self):
        self.p.stdin.close()
        try: self.p.wait(timeout=5)
        except subprocess.TimeoutExpired: self.p.terminate(); self.p.wait(timeout=5)
        assert not self.p.stderr.read(), 'adapter stderr'


def main():
    unknown = Fixture('unknown', protocol='2099-01-01')
    try:
        try:
            bridge.Mcp(unknown.config())
            raise AssertionError('accepted unknown protocol')
        except ValueError:
            pass
        assert not unknown.sessions, 'failed initialize leaked a session'
    finally:
        unknown.close()
    a, b = Fixture('A', protocol='2024-11-05'), Fixture('B', sse=True)
    one, two = Client(), Client()
    try:
        wrong = dict(a.config(), token='incorrect-fixture-token')
        try:
            bridge.Mcp(wrong)
            raise AssertionError('accepted incorrect token')
        except ValueError:
            pass
        m = bridge.Mcp(a.config())
        try:
            a.delay = .2; m.deadline = time.monotonic()+.05
            try:
                m.call('tools/call', {'name': 'browser_title', 'arguments': {}})
                raise AssertionError('ignored call deadline')
            except TimeoutError:
                pass
        finally:
            a.delay = 0; m.close()
        assert len(one.call('tools/list', {})['tools']) == 3
        assert one.tool('current_target')['isError']
        assert bridge.configure(one.binding, a.config())['adapters'] == 1
        assert bridge.configure(two.binding, a.config())['adapters'] == 1
        target_a = one.target()
        assert bridge.configure(one.binding, {'op': 'current'})['targets'] == [target_a]
        assert bridge.configure(one.binding, {'op': 'clear_stale', 'expected_targets': [{'id': 'other-screen', 'revision': 1, 'generation': target_a['generation']}]})['targets'] == [target_a]
        discovery = json.loads(one.tool('list_target_tools')['content'][0]['text'])
        assert discovery['target'] == target_a and any(t['name'] == 'browser_navigate' for t in discovery['tools'])
        one.execute('browser_navigate', {'url': 'https://fixture.example/A'})
        assert 'fixture.example/A' in two.execute('browser_title')['content'][0]['text'], 'shared resources'
        # Selection changes through the real control socket during an in-flight call.
        a.pause = True
        in_flight = {}
        thread = threading.Thread(target=lambda: in_flight.update(result=one.execute('browser_navigate', {'url': 'https://fixture.example/in-flight-A'}, target_a)))
        thread.start(); assert a.started.wait(3)
        try:
            assert bridge.configure(one.binding, b.config())['adapters'] == 1
        finally:
            a.release.set(); thread.join(timeout=10)
        assert not thread.is_alive()
        assert in_flight['result']['_meta']['artTermTarget'] == target_a
        assert 'in-flight-A' in in_flight['result']['content'][0]['text']
        before = len(b.events)
        assert one.execute('browser_navigate', {'url': 'https://wrong.example'}, target_a)['isError']
        assert len(b.events) == before, 'stale target was sent upstream'
        result = one.execute('browser_navigate', {'url': 'https://fixture.example/B'})
        assert '"desktop": "B"' in result['content'][0]['text']
        assert one.execute('fail')['isError'], 'upstream errors must survive'
        rich = one.execute('rich_content')
        assert rich['isError'] is True and [c['type'] for c in rich['content']] == ['image', 'audio', 'resource']
        assert rich['content'][0]['data'] == __import__('base64').b64encode(b.image).decode()
        assert rich['content'][1]['data'] == 'QUJD'
        assert rich['content'][2]['resource']['text'] == 'resource payload'
        assert rich['structuredContent'] == {'source':'B','value':[1,2,3]}
        assert rich['_meta'] == {'vendor':'untouched','artTermTarget':one.target()}
        echoed = one.execute('token_echo')
        assert echoed['isError'] and b.token not in json.dumps(echoed), 'configured token escaped protection'
        assert two.target()['id'] == 'desktop-A'
        assert '"desktop": "A"' in two.execute('browser_title')['content'][0]['text']
        old_two = two.target()
        assert bridge.configure(two.binding, a.config())['adapters'] == 1
        new_two = two.target()
        assert new_two['generation'] > old_two['generation']
        assert bridge.configure(two.binding, {'op': 'clear_stale', 'expected_targets': [old_two]})['targets'] == [new_two]
        assert bridge.configure(two.binding, {'op': 'clear_stale', 'expected_targets': [two.target()]})['targets'] == [None]
        assert two.tool('current_target')['isError']
        assert bridge.configure(one.binding, {'op': 'clear'})['adapters'] == 1
        assert one.execute('browser_title', target=target_a)['isError']
        args = bridge.cli_args(one.binding, ['codex', '--model', 'fixture-model'])
        assert 'mcp_servers.art_term.tool_timeout_sec=75' in args
        assert not any(a.token in x or b.token in x or a.url in x or b.url in x for x in args)
        assert not a.sessions and not b.sessions, 'MCP sessions leaked'
        print('PASS: tool discovery, actual calls, A/B switch, stale-target refusal, shared resource, errors, clear, CLI contract, session cleanup')
    finally:
        one.close(); two.close(); a.close(); b.close()


if __name__ == '__main__': main()
