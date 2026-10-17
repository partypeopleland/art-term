"""Real interactive PTY and fixed-client Shell broker regression (no AI)."""
import fcntl, json, os, pty, select, shlex, signal, struct, subprocess, sys, tempfile, termios, threading, time, uuid
from pathlib import Path
from run import ROOT, bridge, Client
from fixture import Fixture

class Shell:
    def __init__(self, home):
        self.binding='th2-'+uuid.uuid4().hex[:24]
        self.master,slave=pty.openpty()
        self.p=subprocess.Popen([sys.executable,os.environ.get('ART_SHELL_TEST_BRIDGE',str(ROOT/'scripts/instance_bridge.py')),'shell','--binding',self.binding],stdin=slave,stdout=slave,stderr=slave,start_new_session=True,env=dict(os.environ,HOME=str(home),PS1='BROKER_PROMPT> ',ART_TERM_SCREEN_BINDING='',MY_CLI_DESKTOP_MCP_URL='http://retired.invalid/mcp',ART_TERM_DESKTOP_MCP_HELPER='/fixture/retired',HANDS_MCP_TOKEN='retired-fixture-token',GITHUB_TOKEN='unrelated-fixture-value'))
        os.close(slave);self.output=b''
        self.read(b'BROKER_PROMPT> ')
    def read(self, marker, timeout=5):
        end=time.monotonic()+timeout; data=b''
        while marker not in data and time.monotonic()<end:
            if select.select([self.master],[],[],.1)[0]:
                try: chunk=os.read(self.master,65536)
                except OSError: break
                data+=chunk;self.output+=chunk
        assert marker in data,(marker,data[-2000:])
        return data
    def command(self, command, marker=b'BROKER_PROMPT> '):
        done='CMD_DONE_'+uuid.uuid4().hex
        os.write(self.master,(command+"; printf '\\n%s\\n' "+done+'\n').encode())
        return self.read(('\r\n'+done+'\r\n').encode())
    def close(self, sig=None):
        if self.p.poll() is None:
            if sig: self.p.send_signal(sig)
            else: os.write(self.master,b'exit\n')
        self.p.wait(timeout=8);os.close(self.master)


def clients(binding):
    return Client(binding=binding)

def await_dead(pid):
    end=time.monotonic()+5
    while time.monotonic()<end:
        try:
            state=Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()[0]
            if state=='Z':return
        except (FileNotFoundError,ProcessLookupError):return
        time.sleep(.05)
    raise AssertionError(f'owned descendant survived: {pid}')

def main():
    with tempfile.TemporaryDirectory(prefix='shell-broker-') as temp:
        home=Path(temp);(home/'.bashrc').write_text("export PS1='BROKER_PROMPT> '\nexport USER_RC_PRESERVED=yes\n")
        binpath=home/'bin';binpath.mkdir()
        codex=binpath/'codex';codex.write_text('#!/usr/bin/env python3\nimport json,sys\nprint("CODEX_ARGV="+json.dumps(sys.argv[1:]))\n');codex.chmod(0o700)
        s=Shell(home);other=Shell(home)
        a,b=Fixture('A'),Fixture('B',sse=True)
        one=two=None
        try:
            assert (bridge.directory(s.binding)/'shell.sock').exists(), 'broker not available before CLI startup'
            out=s.command('echo SUPERVISED_BASH_PID=$$').decode()
            bash_pid=int(next(line for line in out.splitlines() if line.startswith('SUPERVISED_BASH_PID=')).split('=')[1])
            for owned_pid in (s.p.pid,bash_pid):
                environ=Path(f'/proc/{owned_pid}/environ').read_bytes()
                assert b'MY_CLI_DESKTOP_' not in environ and b'ART_TERM_DESKTOP_' not in environ and b'HANDS_MCP_TOKEN=' not in environ
                assert b'GITHUB_TOKEN=unrelated-fixture-value' in environ
            # Grant before starting any stdio client.
            assert not a.events and not b.events
            assert bridge.configure(s.binding,a.config())['adapters']==1
            one=clients(s.binding);two=clients(other.binding)
            assert one.target()['id']=='desktop-A'
            before=(len(a.events),len(b.events))
            assert two.tool('current_target')['isError']
            assert two.tool('list_target_tools')['isError']
            assert (len(a.events),len(b.events))==before, 'unselected Shell contacted upstream'
            assert 'desktop-A' in s.command('art-term-screen current').decode()
            assert 'browser_navigate' in s.command('art-term-screen list').decode()
            call=json.dumps({'name':'browser_title','arguments':{},'expected_target':one.target()})
            result=s.command("printf '%s' "+shlex.quote(call)+" | art-term-screen call").decode()
            assert 'artTermTarget' in result and 'desktop-A' in result
            assert 'USER_RC=yes' in s.command('echo USER_RC=$USER_RC_PRESERVED').decode()
            s.command('export PATH="'+str(binpath)+':$PATH"')
            yolo=s.command('codex --yolo').decode()
            yolo_line=next(line for line in yolo.splitlines() if line.startswith('CODEX_ARGV='))
            yolo_args=json.loads(yolo_line.split('=',1)[1]);assert yolo_args[-1]=='--yolo' and len(yolo_args)==7
            argvout=s.command('codex --model fixture-only "arg with spaces"')
            line=next(x for x in argvout.decode().splitlines() if x.startswith('CODEX_ARGV='))
            argv=json.loads(line.split('=',1)[1]);assert argv[-3:]==['--model','fixture-only','arg with spaces']
            assert 'mcp_servers.art_term.tool_timeout_sec=75' in argv and s.binding in ''.join(argv)
            assert a.token not in line and b.token not in line
            # Job control: foreground SIGINT, stop/background/foreground, tty resize.
            os.write(s.master,b'sleep 30\n');time.sleep(.15);os.write(s.master,b'\x03');s.read(b'BROKER_PROMPT> ')
            os.write(s.master,b'sleep 30\n');time.sleep(.15);os.write(s.master,b'\x1a');s.read(b'BROKER_PROMPT> ')
            assert 'Stopped' in s.command('jobs').decode()
            s.command('bg');os.write(s.master,b'fg\n');time.sleep(.15);os.write(s.master,b'\x03');s.read(b'BROKER_PROMPT> ')
            fcntl.ioctl(s.master,termios.TIOCSWINSZ,struct.pack('HHHH',37,91,0,0));os.kill(s.p.pid,signal.SIGWINCH)
            time.sleep(.1)
            resized=s.command('stty size').decode();assert '37 91' in resized, resized
            # Late in-flight A result remains A after selection changes to B.
            ta=one.target();a.pause=True;pending={}
            thread=threading.Thread(target=lambda:pending.update(result=one.execute('browser_navigate',{'url':'https://fixture.example/inflight'},ta)))
            thread.start();assert a.started.wait(3)
            bridge.configure(s.binding,b.config());a.release.set();thread.join(10);assert not thread.is_alive()
            assert pending['result']['_meta']['artTermTarget']==ta
            assert one.target()['id']=='desktop-B' and two.tool('current_target')['isError']
            assert one.execute('browser_title',target=ta)['isError']
            # Client EOF does not clear Shell grant; a late client inherits B.
            one.close();one=clients(s.binding);assert one.target()['id']=='desktop-B'
            bridge.configure(s.binding,{'op':'clear'});before=len(b.events)
            assert one.tool('current_target')['isError'];assert one.execute('browser_title',target=ta)['isError'];assert len(b.events)==before
            bridge.configure(s.binding,b.config())
            s.command('sleep 60 & echo OWNED_CHILD=$!')
            pidline=s.command('echo CHILD_PID=$(jobs -p)').decode()
            pid=int(next(x for x in pidline.splitlines() if x.startswith('CHILD_PID=')).split('=')[1])
            # No secrets in work tty, history, child argv/environment or disk.
            hist=s.command('history 100')
            for token in (a.token,b.token):
                assert token.encode() not in s.output+hist
                for proc in Path('/proc').glob('[0-9]*'):
                    try:
                        cmd=(proc/'cmdline').read_bytes()
                        if b'instance_bridge.py' in cmd or int(proc.name)==pid:
                            assert token.encode() not in cmd+(proc/'environ').read_bytes()
                    except (PermissionError,ProcessLookupError,FileNotFoundError):pass
                for path in home.rglob('*'):
                    if path.is_file():assert token.encode() not in path.read_bytes()
            s.close();await_dead(pid)
            assert not (bridge.directory(s.binding)/'shell.sock').exists()
            assert one.tool('current_target')['isError']
            for sig in (signal.SIGTERM,signal.SIGKILL):
                killed=Shell(home)
                out=killed.command('sleep 60 & echo KILL_CHILD=$!').decode()
                child=int(next(x for x in out.splitlines() if x.startswith('KILL_CHILD=')).split('=')[1])
                bridge.configure(killed.binding,a.config())
                killed.close(sig);await_dead(child)
                end=time.monotonic()+5
                while (bridge.directory(killed.binding)/'shell.sock').exists() and time.monotonic()<end:time.sleep(.05)
                assert not (bridge.directory(killed.binding)/'shell.sock').exists()
            assert not a.sessions and not b.sessions
        finally:
            a.release.set()
            if one:one.close()
            if two:two.close()
            if s.p.poll() is None:s.close(signal.SIGTERM)
            other.close();a.close();b.close()
            fast=Shell(home)
            tail=b'FINAL_TAIL_'+uuid.uuid4().hex.encode()
            os.write(fast.master,b"printf '%s' "+tail+b'; exit 7\n')
            fast.p.wait(timeout=8)
            data=b''
            while select.select([fast.master],[],[],.05)[0]:
                try:data+=os.read(fast.master,65536)
                except OSError:break
            os.close(fast.master)
            assert fast.p.returncode==7,fast.p.returncode
            assert data.count(tail)>=2, ('lost final output',data[-200:])
    print('PASS: realPTY Shell selection-beforeCLI, no-grant zeroHTTP, twoShell isolation, scopedCodex argv, bashrc, CtrlC/jobcontrol/resize, A/B source fences, clientEOF, clear, exit/TERM/KILL descendants/socket cleanup, no secrets')
if __name__=='__main__':main()
