# Test server: serves www/ plus /slow (never answers in time) and
# /generate_204. Usage: server.py <root> <port>
import http.server, sys, time, functools
root, port = sys.argv[1], int(sys.argv[2])
class H(http.server.SimpleHTTPRequestHandler):
    def do_GET(self):
        if self.path.startswith('/slow'):
            time.sleep(20)
            self.send_response(200); self.send_header('content-type', 'text/html'); self.end_headers()
            self.wfile.write(b'<title>Real</title>real'); return
        if self.path.startswith('/generate_204'):
            self.send_response(204); self.end_headers(); return
        return super().do_GET()
    def log_message(self, fmt, *args):
        sys.stderr.write("%s %s\n" % (self.address_string(), fmt % args)); sys.stderr.flush()
http.server.ThreadingHTTPServer(('127.0.0.1', port), functools.partial(H, directory=root)).serve_forever()
