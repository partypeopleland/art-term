"""Real public Host helper restart while the previous reader is still alive."""
import argparse,importlib.util,os,pty,fcntl,termios,struct,subprocess,tempfile,secrets,json,time,shutil
from pathlib import Path
root=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('public_host',root/'scripts/linux-regression/tmux_history.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
p=argparse.ArgumentParser();p.add_argument('--lease',action='store_true');args=p.parse_args()
processes=[];master=slave=None;binary=root/'target/tmux-lazy-proof/tmux-3.6a/tmux'
with tempfile.TemporaryDirectory(prefix='tmux-restart-') as folder:
 home=Path(folder);key=secrets.token_hex(12);env=dict(os.environ,HOME=str(home),TERM='xterm-256color')
 support=home/'.local/share/art-term/openab-pty/tmux-v2';support.mkdir(parents=True);(support/'tmux').symlink_to(binary)
 state=home/'.local/share/art-term/tmux-history-v2'/key
 def tmux(*words):return subprocess.check_output([str(binary),'-S',str(state/'tmux.sock'),*words],timeout=3).decode()
 def helper():
  nonce=secrets.token_hex(16)
  proc=subprocess.Popen([str(root/'target/android-tools/node-v22.23.3-linux-x64/bin/node'),str(root/'target/tmux-history/tmux-v2/tmux-history.cjs'),key,nonce],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
  processes.append(proc);reader=m.Reader(proc.stdout)
  return proc,reader,nonce,reader.frame()
 def request(pair,index,op,**fields):
  proc,reader,nonce,_=pair;proc.stdin.write((json.dumps({'v':1,'nonce':nonce,'requestId':index,'op':op,**fields})+'\n').encode());proc.stdin.flush();return reader.frame()
 try:
  master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,80,0,0))
  work=subprocess.Popen(['python3',str(root/'scripts/openab-runtime/tmux-v2/tmux-shell.py'),key],env=env,stdin=slave,stdout=slave,stderr=slave,start_new_session=True);processes.append(work)
  deadline=time.monotonic()+5
  while not (state/'tmux.sock').exists():m.require(time.monotonic()<deadline,'work startup');time.sleep(.02)
  time.sleep(.1);os.write(master,b"printf 'RESTART_ROW\\n'\n");time.sleep(.1)
  first=helper();m.require(first[3]['type']=='ready','first helper ready')
  page=request(first,1,'viewport');m.require(page['type']=='page','first page');request(first,2,'ack',epoch=page['epoch'])
  second=helper();print('SECOND READY:',json.dumps(second[3]),flush=True)
  m.require(second[3]['type']=='ready','new helper must coexist with still-alive previous App helper')
  page2=request(second,1,'viewport');m.require(page2['type']=='page' and any('RESTART_ROW' in r['text'] for r in page2['records']),'same native work history')
  m.require(first[0].poll() is None,'new helper must not terminate another live helper')
  request(second,2,'ack',epoch=page2['epoch'])
  if args.lease:
   deadline=time.monotonic()+130;index=3;invalid_sent=False
   while time.monotonic()<deadline and first[0].poll() is None:
    time.sleep(10);request(second,index,'keep',epoch=page2['epoch']);index+=1
    if index>=9 and not invalid_sent and first[0].poll() is None:
     first[0].stdin.write((json.dumps({'v':1,'nonce':'0'*32,'requestId':3,'op':'viewport'})+'\n').encode());first[0].stdin.flush()
     error=first[1].frame();m.require(error['type']=='error' and error['kind']=='protocol','invalid nonce rejected');invalid_sent=True
   m.require(first[0].poll() is not None,'idle former helper must exit by lease deadline')
   m.require(second[0].poll() is None,'active helper must retain its lease')
   windows=tmux('list-windows','-t','work','-F','#{@art_term_reader}').splitlines()
   m.require(all(first[3]['instance'] not in w for w in windows),'idle helper must remove only its readers')
   m.require(any(second[3]['instance'] in w for w in windows),'active helper reader preserved')
   print('PASS idle lease exit, own cleanup, active helper preserved',flush=True)
  print('PASS public helper App-restart coexistence and same tmux history',flush=True)
 finally:
  for proc in reversed(processes):
   if proc.poll() is None:
    proc.terminate()
    try:proc.wait(timeout=5)
    except subprocess.TimeoutExpired:proc.kill();proc.wait()
  if (state/'tmux.sock').exists():subprocess.run([str(binary),'-S',str(state/'tmux.sock'),'kill-server'],capture_output=True)
  for fd in (master,slave):
   if fd is not None:os.close(fd)
