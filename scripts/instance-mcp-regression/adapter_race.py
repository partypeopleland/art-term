"""A late configure failure must not clear a newer confirmed target."""
import threading
from fixture import Fixture
from run import Client, bridge

def main():
    a,b = Fixture('A'),Fixture('B')
    client = Client()
    pending = {}
    def configure_old():
        try: pending['result'] = bridge.configure(client.binding,a.config())
        except ValueError: pending['rejected'] = True
    thread = threading.Thread(target=configure_old)
    try:
        a.pause_list = True
        thread.start(); assert a.list_started.wait(3)
        assert bridge.configure(client.binding,b.config())['adapters'] == 1
        confirmed = client.target()
        assert confirmed['id'] == 'desktop-B'
        a.list_release.set(); thread.join(10); assert not thread.is_alive()
        assert pending.get('rejected'), 'older configure unexpectedly committed'
        current = client.tool('current_target')
        assert not current.get('isError'), 'stale configure rollback cleared newer confirmed B'
        assert client.target() == confirmed
        assert client.execute('browser_title')['_meta']['artTermTarget'] == confirmed
        # An explicit user clear still invalidates an older pending configure.
        a.pause_list = True; a.list_started.clear(); a.list_release.clear(); pending.clear()
        thread = threading.Thread(target=configure_old); thread.start(); assert a.list_started.wait(3)
        bridge.configure(client.binding,{'op':'clear'})
        a.list_release.set(); thread.join(10); assert not thread.is_alive()
        assert client.tool('current_target')['isError']
        print('PASS: stale-configure rollback preserves newer target; explicit clear cancels pending configure')
    finally:
        a.list_release.set(); thread.join(10); client.close(); a.close(); b.close()

if __name__ == '__main__': main()
