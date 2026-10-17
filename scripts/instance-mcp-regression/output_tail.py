"""Exercise final drain's actual PTY EIO with already queued terminal bytes."""
import ast, errno, os, pty, select, sys, tempfile, time
from pathlib import Path

path=Path(sys.argv[1]) if len(sys.argv)>1 else Path(__file__).resolve().parents[1]/'instance-shell.py'
module=ast.parse(path.read_text());shell=next(node for node in module.body if isinstance(node,ast.FunctionDef) and node.name=='shell')
try_node=next(node for node in shell.body if isinstance(node,ast.Try))
drain=next(node for node in reversed(try_node.body) if isinstance(node,ast.While))
master,slave=pty.openpty();os.close(slave)
original=os.dup(1)
try:
    with tempfile.TemporaryFile() as output:
        os.dup2(output.fileno(),1)
        queued=bytearray(b'FINAL_QUEUED_TAIL\x1b[0m\r\n')
        context={'time':time,'select':select,'os':os,'errno':errno,'master':master,'outgoing':queued,'end':time.monotonic()+.3,'eof':False}
        exec(compile(ast.Module(body=[drain],type_ignores=[]),str(path),'exec'),context)
        output.seek(0);actual=output.read()
        assert actual==b'FINAL_QUEUED_TAIL\x1b[0m\r\n',actual
finally:
    os.dup2(original,1);os.close(original);os.close(master)
print('PASS: actual PTY EOF/EIO flushes queued final bytes including ANSI/CRLF')
