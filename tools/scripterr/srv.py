import http.server, os
class H(http.server.SimpleHTTPRequestHandler):
    def do_GET(self):
        if self.path.startswith('/missing.js'):
            body = b"document.getElementById('out').textContent += ' BAD404';"
            self.send_response(404); self.send_header('Content-Type','application/javascript')
            self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body); return
        if self.path.startswith('/err500.js'):
            body = b"document.getElementById('out').textContent += ' BAD500';"
            self.send_response(500); self.send_header('Content-Type','application/javascript')
            self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body); return
        return super().do_GET()
    def log_message(self,*a): pass
os.chdir(os.path.dirname(os.path.abspath(__file__)))
http.server.ThreadingHTTPServer(('127.0.0.1', 8733), H).serve_forever()
