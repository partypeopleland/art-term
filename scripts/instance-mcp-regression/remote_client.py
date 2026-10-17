"""Fixed AI client running INSIDE the official Runtime work Shell."""
import argparse
import socketserver
import sys
from run import Client, bridge

parser = argparse.ArgumentParser()
parser.add_argument('--binding', required=True)
parser.add_argument('--socket', required=True)
args = parser.parse_args()
client = Client(args.binding)

class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        request = bridge.receive(self.request)
        result = client.call(request['method'], request.get('params', {}))
        bridge.send(self.request, result)

server = socketserver.UnixStreamServer(args.socket, Handler)
print('FIXED_AI_READY', flush=True)
try: server.serve_forever()
finally: client.close(); server.server_close()
