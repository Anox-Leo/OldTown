"""Connexion IMAP persistante en IDLE : appelle `on_message` pour chaque nouveau mail.

Boucle d'une session :
    SELECT (lecture seule) → rattrapage des UID manqués → IDLE
    → dès qu'un « * N EXISTS » arrive : DONE, UID FETCH des nouveaux messages → IDLE…
L'IDLE est renouvelé toutes les `idle_refresh` secondes (Gmail le coupe vers 29 min),
ce qui sert aussi de test de vie de la connexion, avec le keepalive TCP.
"""

from __future__ import annotations

import imaplib
import logging
import re
import socket
import ssl
import threading
import time
from collections.abc import Callable

from .config import ImapConfig

log = logging.getLogger(__name__)

FETCH_LIMIT = 512 * 1024  # un mail d'OTP fait quelques dizaines de Ko ; on tronque les gros
STARTUP_LOOKBACK = 20  # nombre de derniers messages examinés au démarrage

OnMessage = Callable[[bytes, float, int], None]  # (mail brut, reçu à [epoch], uid)
OnStatus = Callable[[str, str], None]  # (état, détail)


class AuthError(Exception):
    pass


class ImapWatcher(threading.Thread):
    def __init__(self, cfg: ImapConfig, password: str, on_message: OnMessage, on_status: OnStatus) -> None:
        super().__init__(name="imap-watcher", daemon=True)
        self.cfg = cfg
        self._password = password
        self.on_message = on_message
        self.on_status = on_status
        self._stop = threading.Event()
        self._imap: imaplib.IMAP4 | None = None
        self._uidvalidity: int | None = None
        self._last_uid: int | None = None

    # ------------------------------------------------------------------ cycle de vie

    def stop(self) -> None:
        self._stop.set()
        imap = self._imap
        if imap is not None and imap.sock is not None:
            try:  # débloque la lecture en cours dans l'IDLE
                imap.sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass

    def run(self) -> None:
        backoff = 1.0
        while not self._stop.is_set():
            started = time.monotonic()
            try:
                self._session()
            except AuthError as exc:
                log.error("authentification IMAP refusée : %s", exc)
                self.on_status("auth_failed", str(exc))
                # Insister sur un mot de passe faux peut verrouiller le compte Google.
                self._stop.wait(900)
                continue
            except (imaplib.IMAP4.error, OSError, ssl.SSLError, EOFError) as exc:
                if self._stop.is_set():
                    break
                log.warning("connexion IMAP perdue : %s: %s", type(exc).__name__, exc)
                self.on_status("disconnected", f"{type(exc).__name__}: {exc}")
            finally:
                self._close()
            if time.monotonic() - started > 120:
                backoff = 1.0  # la session a tenu : on repart vite
            log.info("reconnexion dans %.0f s", backoff)
            self._stop.wait(backoff)
            backoff = min(backoff * 2, 300)
        self.on_status("stopped", "")

    def _close(self) -> None:
        imap, self._imap = self._imap, None
        if imap is None:
            return
        try:
            imap.logout()
        except Exception:
            pass

    # ------------------------------------------------------------------ session

    def _connect(self) -> imaplib.IMAP4:
        cfg = self.cfg
        timeout = cfg.connect_timeout
        if cfg.security == "ssl":
            imap: imaplib.IMAP4 = imaplib.IMAP4_SSL(
                cfg.host, cfg.port, ssl_context=ssl.create_default_context(), timeout=timeout
            )
        else:
            imap = imaplib.IMAP4(cfg.host, cfg.port, timeout=timeout)
            if cfg.security == "starttls":
                imap.starttls(ssl_context=ssl.create_default_context())
        sock = imap.sock
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_KEEPALIVE, 1)
        if hasattr(socket, "TCP_KEEPIDLE"):  # connexion morte détectée en ~2 min au lieu de ~2 h
            sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_KEEPIDLE, 60)
            sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_KEEPINTVL, 15)
            sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_KEEPCNT, 4)
        return imap

    def _session(self) -> None:
        cfg = self.cfg
        imap = self._imap = self._connect()
        if self._stop.is_set():
            return
        try:
            imap.login(cfg.username, self._password)
        except imaplib.IMAP4.error as exc:
            raise AuthError(str(exc)) from None
        if "IDLE" not in imap.capabilities:
            raise imaplib.IMAP4.error("le serveur ne supporte pas IDLE")

        typ, data = imap.select(_quote(cfg.mailbox), readonly=True)
        if typ != "OK":
            raise imaplib.IMAP4.error(f"SELECT {cfg.mailbox} : {data}")
        uidvalidity = int(imap.untagged_responses.get("UIDVALIDITY", [b"0"])[-1])
        uidnext = int(imap.untagged_responses.get("UIDNEXT", [b"1"])[-1])

        if self._last_uid is None or uidvalidity != self._uidvalidity:
            # Premier démarrage (ou boîte réinitialisée) : on repart des derniers messages
            # d'hier et d'aujourd'hui ; seuls ceux de moins de max_age seront diffusés.
            # (Les UID Gmail ne sont pas contigus : impossible de reculer de N UID.)
            self._uidvalidity = uidvalidity
            self._last_uid = uidnext - 1
            typ, data = imap.uid("SEARCH", None, "SINCE", _imap_date(time.time() - 86400))
            recent = sorted(map(int, (data[0] or b"").split())) if typ == "OK" else []
            if recent:
                self._last_uid = recent[-STARTUP_LOOKBACK:][0] - 1
        log.info("connecté à %s, %s (UIDNEXT=%d)", cfg.host, cfg.mailbox, uidnext)
        self.on_status("connected", f"{cfg.host}/{cfg.mailbox}")

        while not self._stop.is_set():
            imap.untagged_responses.clear()  # sinon EXPUNGE/FETCH s'accumulent sur des jours
            self._fetch_new(imap)
            if self._wait_for_mail(imap):
                log.debug("nouveau message signalé")

    def _wait_for_mail(self, imap: imaplib.IMAP4) -> bool:
        """Bloque en IDLE ; True dès qu'un nouveau message est signalé, False au renouvellement."""
        with imap.idle(duration=self.cfg.idle_refresh) as idler:
            for typ, data in idler:
                if typ == "EXISTS":
                    return True
                if typ == "BYE":
                    raise imaplib.IMAP4.abort(f"BYE pendant IDLE : {data}")
        return False

    def _fetch_new(self, imap: imaplib.IMAP4) -> None:
        assert self._last_uid is not None
        typ, data = imap.uid("SEARCH", None, f"UID {self._last_uid + 1}:*")
        if typ != "OK":
            raise imaplib.IMAP4.error(f"UID SEARCH : {data}")
        # « N:* » renvoie toujours le dernier message, même si son UID < N.
        uids = sorted(u for u in map(int, (data[0] or b"").split()) if u > self._last_uid)
        if not uids:
            return
        typ, data = imap.uid(
            "FETCH", ",".join(map(str, uids)), f"(UID INTERNALDATE BODY.PEEK[]<0.{FETCH_LIMIT}>)"
        )
        if typ != "OK":
            raise imaplib.IMAP4.error(f"UID FETCH : {data}")

        now = time.time()
        for meta, raw in _fetch_items(data):
            uid_match = re.search(rb"UID (\d+)", meta)
            if not uid_match:
                continue
            uid = int(uid_match[1])
            date = imaplib.Internaldate2tuple(meta)
            received_at = time.mktime(date) if date else now
            self._last_uid = max(self._last_uid, uid)
            age = now - received_at
            if age > self.cfg.max_age:
                log.debug("uid %d ignoré : reçu il y a %.0f s", uid, age)
                continue
            try:
                self.on_message(raw, received_at, uid)
            except Exception:
                log.exception("erreur pendant le traitement du message uid %d", uid)
        self._last_uid = max(self._last_uid, uids[-1])


def _fetch_items(data: list) -> list[tuple[bytes, bytes]]:
    """Regroupe la réponse d'imaplib en (méta-données, corps) par message.

    imaplib renvoie pour chaque message un tuple (préfixe, littéral) suivi d'un
    fragment b' INTERNALDATE ...)' quand le serveur place des attributs après le corps.
    """
    items: list[tuple[bytes, bytes]] = []
    for part in data:
        if isinstance(part, tuple):
            items.append((part[0], part[1]))
        elif isinstance(part, bytes) and items:
            meta, raw = items[-1]
            items[-1] = (meta + b" " + part, raw)
    return items


def _imap_date(epoch: float) -> str:
    t = time.gmtime(epoch)
    month = ("Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec")[t.tm_mon - 1]
    return f"{t.tm_mday:02d}-{month}-{t.tm_year}"


def _quote(mailbox: str) -> str:
    if re.fullmatch(r"[A-Za-z0-9_./\[\]-]+", mailbox):
        return mailbox
    return '"' + mailbox.replace("\\", "\\\\").replace('"', '\\"') + '"'
