"""Real Runtime + th2 + Node22 reader snapshots must not outlive work Shell exit."""
import hashlib,json,os,shutil,socket,subprocess,tempfile,time,uuid
from pathlib import Path
from official_runtime import ROOT,linux

def main():
    root=Path(tempfile.mkdtemp(prefix='art-he-'));h=linux.Harness(root,None);user_socket=None;td=None
    try:
     tools=h.home/'.local/share/art-term/tools';tools.mkdir(parents=True,mode=0o700)
     for name in ('instance_bridge.py','instance-shell.py','instance-shell.bash'):shutil.copyfile(ROOT/'scripts'/name,tools/name)
     shutil.copyfile(ROOT/'scripts/openab-runtime/tmux-v2/tmux-shell.py',tools/'tmux-shell.py')
     td=h.helper/'tmux-v2';td.mkdir(mode=0o700)
     shutil.copyfile(Path.home()/'.local/share/art-term/openab-pty/tmux-v2/tmux',td/'tmux');(td/'tmux').chmod(0o700)
     shutil.copyfile(ROOT/'target/android-tools/node-v22.23.3-linux-x64/bin/node',h.helper/'node');(h.helper/'node').chmod(0o700)
     shutil.copyfile(ROOT/'target/tmux-history/tmux-v2/tmux-history.cjs',td/'tmux-history.cjs')
     (h.home/'.bashrc').write_text("PS1='WORK_READY> '\n")
     key=uuid.uuid4().hex[:24];nonce=uuid.uuid4().hex
     work,grant=h.create('work');ws=h.attach(work,grant)
     ws.send(2,f'exec python3 {tools}/tmux-shell.py {key}\n'.encode());ws.collect(b'WORK_READY> ')
     helper,hgrant=h.create('mcshot-');reader=h.attach(helper,hgrant)
     reader.send(2,f'exec {h.helper}/node {td}/tmux-history.cjs {key} {nonce}\n'.encode());reader.collect(b'"type":"ready"')
     reader.send(2,(json.dumps({'v':1,'nonce':nonce,'requestId':1,'op':'viewport'})+'\n').encode())
     frame=reader.collect(b'"type":"page"');assert b'"type":"page"' in frame
     import re
     epoch=int(re.search(rb'"epoch":(\d+)',frame)[1])
     reader.send(2,(json.dumps({'v':1,'nonce':nonce,'requestId':2,'op':'ack','epoch':epoch})+'\n').encode());reader.collect(b'"op":"ack"')
     state=h.home/'.local/share/art-term/tmux-history-v2'/key/'tmux.sock'
     import subprocess
     def windows():return subprocess.check_output([str(td/'tmux'),'-S',str(state),'list-windows','-t','work','-F','#{window_id}:#{window_name}:#{@art_term_reader}'],timeout=3).decode().splitlines()
     assert subprocess.check_output([str(td/'tmux'),'-S',str(state),'display-message','-p','-t','work:0.0','#{pane_id}'],timeout=3).strip()==b'%0','fresh private authoritative work pane was not %0'
     before=windows();print('before exit',json.dumps(before),flush=True)
     assert len(before)>1,'real reader did not create snapshot window'
     # A reader-only window exit must never terminate the authoritative work pane.
     for window in before:
      if window.split(':',2)[2]:subprocess.run([str(td/'tmux'),'-S',str(state),'kill-window','-t',window.split(':',1)[0]],check=True)
     assert len(windows())==1 and next(s for s in h.api('GET','/admin/sessions')[1]['sessions'] if s['name']==work)['alive']
     time.sleep(.55)
     reader.send(2,(json.dumps({'v':1,'nonce':nonce,'requestId':3,'op':'viewport'})+'\n').encode());reader.collect(b'"type":"page"')
     assert len(windows())>1
     # Foreground CLI exit only returns to Bash.
     ws.send(2,b"bash -c 'exit 7'; printf 'CLI_%s\\n' RETURNED\n");ws.collect(b'CLI_RETURNED')
     assert next(s for s in h.api('GET','/admin/sessions')[1]['sessions'] if s['name']==work)['alive']
     key2=uuid.uuid4().hex[:24];other,g2=h.create('other');ws2=h.attach(other,g2)
     ws2.send(2,f'exec python3 {tools}/tmux-shell.py {key2}\n'.encode());ws2.collect(b'WORK_READY> ')
     state2=h.home/'.local/share/art-term/tmux-history-v2'/key2/'tmux.sock'
     assert subprocess.check_output([str(td/'tmux'),'-S',str(state2),'display-message','-p','-t','work:0.0','#{pane_id}'],timeout=3).strip()==b'%0'
     user_socket=root/'unrelated.sock'
     subprocess.run([str(td/'tmux'),'-S',str(user_socket),'-f','/dev/null','new-session','-d','-s','unrelated','sleep 60'],check=True)
     ws.send(2,b'exit\n');deadline=time.monotonic()+2;closed=False
     while time.monotonic()<deadline:
      try:op,raw=ws.recv(deadline-time.monotonic())
      except (socket.timeout,TimeoutError):break
      if op==8:
       closed=int.from_bytes(raw[:2],'big')==4004;break
     if not closed:
      print('after exit',json.dumps(windows()),flush=True)
      print('Runtime alive',json.dumps([{'name':s['name'],'alive':s['alive']} for s in h.api('GET','/admin/sessions')[1]['sessions']]),flush=True)
     assert closed,'work Bash exited but reader snapshot kept Runtime work Shell alive (no4004 within2s)'
     broker=Path('/tmp')/('art-term-'+str(os.getuid()))/hashlib.sha256(('th2-'+key).encode()).hexdigest()[:24]/'shell.sock'
     assert not broker.exists(),'exited work Shell broker socket leaked'
     assert next(s for s in h.api('GET','/admin/sessions')[1]['sessions'] if s['name']==other)['alive']
     subprocess.run([str(td/'tmux'),'-S',str(user_socket),'has-session','-t','unrelated'],check=True)
     ws2.send(2,b'exit 7\n');op=0;end=time.monotonic()+2
     while op!=8 and time.monotonic()<end:op,raw=ws2.recv(end-time.monotonic())
     assert op==8 and int.from_bytes(raw[:2],'big')==4004
     print('PASS: reader-only cleanup/CLI exit keep work alive; other private and unrelated namespaces untouched; exit7 closes; real Node22 viewport snapshot cannot retain exited work Shell; official Runtime4004 within2s')
    finally:
     if user_socket is not None and td is not None:
      subprocess.run([str(td/'tmux'),'-S',str(user_socket),'kill-session','-t','unrelated'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
     h.close()

if __name__ == '__main__':
    main()
