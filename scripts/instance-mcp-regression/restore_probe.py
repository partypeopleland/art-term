"""Restoration observes modern and already-running legacy brokers without mutation."""
import json
import os
import tempfile
import threading
from fixture import Fixture
from run import bridge


class LegacyBroker(bridge.Broker):
    def configure(self, body):
        if body.get('op') in ('current', 'clear_stale'):
            raise ValueError(bridge.ERROR)
        return super().configure(body)


def probe(binding, target):
    return bridge.configure(binding, {'op': 'verify', 'expected_targets': [target]})


def main():
    a, b = Fixture('A'), Fixture('B')
    old_runtime = os.environ.get('XDG_RUNTIME_DIR')
    with tempfile.TemporaryDirectory() as root:
        os.environ['XDG_RUNTIME_DIR'] = root
        brokers = []
        try:
            for kind in (bridge.Broker, LegacyBroker):
                binding = 'restore-' + kind.__name__.lower()
                broker = kind()
                server, path = bridge.serve(binding, broker, shell=True)
                brokers.append((server, path))
                target = broker.configure(a.config())['target']
                result = probe(binding, target)
                assert result == {'ok': True, 'adapters': 1, 'targets': [target], 'urls': [None]}
                encoded = json.dumps(result)
                assert a.token not in encoded and a.url not in encoded, 'probe exposed credentials or upstream URL'
                assert broker.selection['config'] == {k: v for k, v in a.config().items() if k != 'op'}, 'restore changed broker credentials or identity'
                assert broker.selection['generation'] == target['generation'], 'restore changed generation'
                current = bridge.configure(binding, {'op': 'current'})
                assert current['targets'] == [target], 'legacy current fallback lost assignment'
                assert (current['urls'][0] is None) == (kind is LegacyBroker)
                a.pause_list = True
                a.list_started.clear()
                a.list_release.clear()
                pending = {}
                def verify():
                    try:
                        probe(binding, target)
                    except ValueError:
                        pending['rejected'] = True
                thread = threading.Thread(target=verify)
                thread.start()
                assert a.list_started.wait(3)
                newer = broker.configure(b.config())['target']
                a.list_release.set()
                thread.join(10)
                assert not thread.is_alive()
                assert pending.get('rejected'), 'probe accepted target changed while upstream was checked'
                assert broker.selection['config'] == {k: v for k, v in b.config().items() if k != 'op'}, 'failed restore overwrote newer selection'
                assert broker.selection['generation'] == newer['generation']
                a.pause_list = False
                broker.selection['config']['token'] = 'invalid-fictional-token'
                try:
                    probe(binding, newer)
                    raise AssertionError('probe claimed ready with invalid broker credentials')
                except ValueError:
                    pass
                assert broker.selection['generation'] == newer['generation'], 'failed credential probe cleared assignment'
                assert broker.selection['config']['id'] == b.config()['id']
            print('PASS: modern and legacy restore probes preserve live assignments and reject changed generation')
        finally:
            a.list_release.set()
            for server, path in brokers:
                server.shutdown()
                server.server_close()
                path.unlink(missing_ok=True)
            if old_runtime is None:
                os.environ.pop('XDG_RUNTIME_DIR', None)
            else:
                os.environ['XDG_RUNTIME_DIR'] = old_runtime
            a.close()
            b.close()


if __name__ == '__main__':
    main()
