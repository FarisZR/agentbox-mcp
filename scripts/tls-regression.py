#!/usr/bin/env python3
"""Exercise the real OAuth/JWKS HTTPS client using synthetic local chains."""

import http.server
import os
from pathlib import Path
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request


FIXTURES = Path(__file__).resolve().parent.parent / "tests/fixtures/tls"


class JwksHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = b'{"keys":[]}'
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args):
        pass


def check(binary, fixture, should_start, hostname="localhost"):
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), JwksHandler)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(
        FIXTURES / f"{fixture}-chain.pem", FIXTURES / f"{fixture}-test-key.pem"
    )
    server.socket = context.wrap_socket(server.socket, server_side=True)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix="agentbox-jwks-tls-") as tmp:
            tmp = Path(tmp)
            with socket.socket() as port_socket:
                port_socket.bind(("127.0.0.1", 0))
                app_port = port_socket.getsockname()[1]
            config = tmp / "config.toml"
            config.write_text(
                f'[server]\nbind = "127.0.0.1:{app_port}"\n'
                '[auth]\nmode = "oauth_jwks"\n[auth.oauth]\n'
                f'jwks_url = "https://{hostname}:{server.server_port}/jwks"\n'
            )
            env = os.environ.copy()
            # This isolated child trusts our test root. Never modify the runner's
            # global environment or bypass a remote network policy/proxy.
            env["SSL_CERT_FILE"] = str(FIXTURES / "root.pem")
            env["SSL_CERT_DIR"] = str(tmp)
            env["NO_PROXY"] = env["no_proxy"] = "localhost,127.0.0.1"
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            with (tmp / "server.log").open("w+") as log:
                child = subprocess.Popen(
                    [str(binary), "--config", str(config)],
                    env=env,
                    stdout=log,
                    stderr=subprocess.STDOUT,
                )
                try:
                    started = False
                    deadline = time.monotonic() + 15
                    while time.monotonic() < deadline and child.poll() is None:
                        try:
                            with opener.open(
                                f"http://127.0.0.1:{app_port}/healthz", timeout=0.2
                            ) as response:
                                started = response.status == 200
                            if started:
                                break
                        except OSError:
                            pass
                        time.sleep(0.05)
                    log.seek(0)
                    output = log.read()
                    assert started == should_start, (
                        f"{fixture}/{hostname}: started={started}, "
                        f"expected={should_start}\n{output}"
                    )
                    if not should_start:
                        assert child.poll() not in (None, 0), output
                        assert "fetch jwks" in output, output
                        if fixture != "valid":
                            assert "distrust" in output.lower(), output
                finally:
                    if child.poll() is None:
                        child.terminate()
                        try:
                            child.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            child.kill()
                            child.wait()
    finally:
        server.shutdown()
        server.server_close()
    print(f"TLS JWKS {fixture}/{hostname}: ok")


if __name__ == "__main__":
    binary = Path(sys.argv[1]).resolve()
    check(binary, "valid", True)
    check(binary, "distrusted", False)
    check(binary, "reissued", False)
    check(binary, "valid", False, hostname="127.0.0.1")
