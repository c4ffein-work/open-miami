#!/usr/bin/env python3
"""Static dev server with aggressive no-cache headers, so a reverse proxy / CDN
in front of the container never serves stale index.html / .js / .wasm."""
import http.server, socketserver, os
os.chdir(os.path.dirname(os.path.abspath(__file__)))

class NoCache(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cache-Control", "no-store, no-cache, must-revalidate, max-age=0")
        self.send_header("Pragma", "no-cache")
        self.send_header("Expires", "0")
        super().end_headers()

socketserver.TCPServer.allow_reuse_address = True
with socketserver.TCPServer(("0.0.0.0", 8080), NoCache) as httpd:
    print("serving :8080 (no-store)")
    httpd.serve_forever()
