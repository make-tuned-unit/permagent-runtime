#!/usr/bin/env python3
"""Serve this fixture on loopback only; does not open a browser or play audio."""
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from functools import partial
from pathlib import Path
import argparse
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--port',type=int,default=5489)
a=p.parse_args()
if not 1024<=a.port<=65535:p.error('port must be 1024..65535')
root=Path(__file__).resolve().parent
class Handler(SimpleHTTPRequestHandler):
 def end_headers(self):
  self.send_header('Cache-Control','no-store')
  self.send_header('X-Content-Type-Options','nosniff')
  super().end_headers()
server=ThreadingHTTPServer(('127.0.0.1',a.port),partial(Handler,directory=str(root)))
print(f'Practice fixture: http://127.0.0.1:{a.port}/ (no playback until Start)',flush=True)
try:server.serve_forever()
except KeyboardInterrupt:pass
finally:server.server_close()
