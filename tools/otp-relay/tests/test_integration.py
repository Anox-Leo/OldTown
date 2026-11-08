"""Test de bout en bout contre un vrai serveur IMAP (désactivé par défaut).

    docker run -d --name otp-relay-it -p 127.0.0.1:11143:143 \
        -v $PWD/tests/dovecot-plain.conf:/etc/dovecot/conf.d/zz-plain.conf:ro dovecot/dovecot:2.3-latest
    OTP_RELAY_IT=127.0.0.1:11143:test:pass python3 -m unittest tests.test_integration -v
"""

import imaplib
import json
import os
import socket
import subprocess
import sys
import tempfile
import time
import unittest
from email.message import EmailMessage
from pathlib import Path

TARGET = os.environ.get("OTP_RELAY_IT")
ROOT = Path(__file__).resolve().parent.parent


def otp_mail(code: str, subject: str = "Votre code de connexion") -> bytes:
    msg = EmailMessage()
    msg["From"] = "Exemple <no-reply@exemple.fr>"
    msg["To"] = "test@localhost"
    msg["Subject"] = subject
    msg["Message-ID"] = f"<{time.time_ns()}@exemple.fr>"
    msg["Authentication-Results"] = "mx.google.com; dkim=pass; spf=pass; dmarc=pass header.from=exemple.fr"
    msg.set_content(f"Bonjour,\n\nVotre code de vérification : {code}\n\nIl expire dans 10 minutes.")
    return bytes(msg)


@unittest.skipUnless(TARGET, "définir OTP_RELAY_IT=hôte:port:utilisateur:mot_de_passe")
class EndToEnd(unittest.TestCase):
    def setUp(self):
        host, port, self.user, self.password = TARGET.split(":")
        self.host, self.port = host, int(port)
        self.tmp = tempfile.TemporaryDirectory()
        tmp = Path(self.tmp.name)
        self.sock_path = tmp / "run" / "otp.sock"
        config = tmp / "config.toml"
        config.write_text(
            f'[imap]\nhost = "{host}"\nport = {port}\nsecurity = "none"\nusername = "{self.user}"\n'
            f'idle_refresh = 60\n[socket]\npath = "{self.sock_path}"\n[log]\nlevel = "DEBUG"\n'
        )
        env = dict(os.environ, OTP_RELAY_IMAP_PASSWORD=self.password, PYTHONPATH=str(ROOT))
        self.proc = subprocess.Popen([sys.executable, "-m", "otp_relay", "run", "-c", str(config)],
                                     env=env, stderr=subprocess.PIPE, text=True)
        self.addCleanup(self._stop)

    def _stop(self):
        self.proc.terminate()
        try:
            _, err = self.proc.communicate(timeout=10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            _, err = self.proc.communicate()
        if os.environ.get("OTP_RELAY_IT_LOGS"):
            print(err, file=sys.stderr)
        self.tmp.cleanup()

    def _client(self):
        for _ in range(50):
            if self.sock_path.exists():
                break
            time.sleep(0.1)
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.connect(str(self.sock_path))
        s.settimeout(10)
        return s, s.makefile("r")

    def _wait(self, lines, predicate):
        while True:
            msg = json.loads(lines.readline())
            if predicate(msg):
                return msg

    def _append(self, raw: bytes):
        m = imaplib.IMAP4(self.host, self.port)
        m.login(self.user, self.password)
        m.append("INBOX", None, imaplib.Time2Internaldate(time.time()), raw)
        m.logout()

    def test_code_is_pushed_within_a_second(self):
        sock, lines = self._client()
        self._wait(lines, lambda m: m["type"] == "status" and m["state"] == "connected")
        time.sleep(0.3)  # laisse le daemon entrer en IDLE

        latencies = []
        for code in ("482913", "105577", "930214"):
            t0 = time.monotonic()
            self._append(otp_mail(code))
            msg = self._wait(lines, lambda m: m["type"] == "otp")
            latencies.append(time.monotonic() - t0)
            self.assertEqual(msg["code"], code)
            self.assertTrue(msg["sender_verified"])
            self.assertEqual(msg["sender"]["domain"], "exemple.fr")

        self._append(otp_mail("", subject="Newsletter").replace(b"code", b"texte"))
        print(f"\nlatences APPEND → événement : {', '.join(f'{x * 1000:.0f} ms' for x in latencies)}")
        self.assertLess(max(latencies), 1.0)

        # Un second client reçoit les codes encore valides en replay
        sock2, lines2 = self._client()
        replayed = [m["code"] for m in (json.loads(lines2.readline()) for _ in range(5)) if m["type"] == "otp"]
        self.assertEqual(sorted(replayed), ["105577", "482913", "930214"])
        sock.close()
        sock2.close()

    def test_reconnects_after_server_restart(self):
        if not os.environ.get("OTP_RELAY_IT_DOCKER"):
            self.skipTest("définir OTP_RELAY_IT_DOCKER=<conteneur> pour tester la reconnexion")
        sock, lines = self._client()
        self._wait(lines, lambda m: m["type"] == "status" and m["state"] == "connected")
        subprocess.run(["docker", "restart", "-t", "0", os.environ["OTP_RELAY_IT_DOCKER"]], check=True,
                       capture_output=True)
        self._wait(lines, lambda m: m["type"] == "status" and m["state"] == "disconnected")
        self._wait(lines, lambda m: m["type"] == "status" and m["state"] == "connected")
        time.sleep(0.3)
        self._append(otp_mail("774411"))
        self.assertEqual(self._wait(lines, lambda m: m["type"] == "otp")["code"], "774411")
        sock.close()


if __name__ == "__main__":
    unittest.main()
