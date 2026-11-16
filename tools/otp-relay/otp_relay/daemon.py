"""Assemblage : watcher IMAP (thread) → extraction → hub (boucle asyncio)."""

from __future__ import annotations

import asyncio
import logging
import os
import signal
import socket
import time
from typing import Any

from .config import Config
from .extract import detect, parse
from .hub import Hub, iso
from .imap_watcher import ImapWatcher

log = logging.getLogger(__name__)


def redact(code: str) -> str:
    return code[:2] + "•" * (len(code) - 2)


def build_event(raw: bytes, received_at: float, uid: int, cfg: Config) -> tuple[dict[str, Any], float] | None:
    mail = parse(raw)
    found = detect(mail)
    if found is None:
        log.debug("uid %d (%s) : aucun code", uid, mail.from_domain)
        return None
    if found.confidence < cfg.min_confidence:
        log.info("uid %d (%s) : code écarté, confiance %.2f", uid, mail.from_domain, found.confidence)
        return None

    now = time.time()
    expires = max(received_at + cfg.code_ttl, now + 60)
    event = {
        "type": "otp",
        "id": mail.event_id,
        "code": found.code,
        "display": found.display,
        "confidence": found.confidence,
        "sender": {"name": mail.from_name, "address": mail.from_address, "domain": mail.from_domain},
        "sender_verified": mail.auth.get("dmarc") == "pass",
        "auth": mail.auth,
        "to": mail.to_address,
        "subject": mail.subject,
        "received_at": iso(received_at),
        "detected_at": iso(now),
        "expires_at": iso(expires),
        "source": {"mailbox": cfg.imap.mailbox, "uid": uid},
    }
    shown = found.code if cfg.log_codes else redact(found.code)
    log.info("code %s de %s (confiance %.2f, DMARC %s, %.1f s après réception)",
             shown, mail.from_domain, found.confidence, mail.auth.get("dmarc", "?"), now - received_at)
    return event, expires


def sd_notify(state: str) -> None:
    """Protocole sd_notify de systemd (Type=notify), sans dépendance."""
    addr = os.environ.get("NOTIFY_SOCKET")
    if not addr:
        return
    if addr.startswith("@"):
        addr = "\0" + addr[1:]
    with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as sock:
        try:
            sock.sendto(state.encode(), addr)
        except OSError:
            pass


async def run(cfg: Config) -> None:
    password = cfg.imap.password()
    loop = asyncio.get_running_loop()
    hub = Hub(cfg.socket, cfg.code_ttl)
    await hub.start()

    def on_message(raw: bytes, received_at: float, uid: int) -> None:
        result = build_event(raw, received_at, uid, cfg)
        if result is not None:
            loop.call_soon_threadsafe(hub.publish, *result)

    def on_status(state: str, detail: str) -> None:
        loop.call_soon_threadsafe(hub.set_status, state, detail)
        sd_notify(f"STATUS=IMAP {state}")

    watcher = ImapWatcher(cfg.imap, password, on_message, on_status)
    watcher.start()
    sd_notify("READY=1")

    stop = asyncio.Event()
    for sig in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(sig, stop.set)
    await stop.wait()

    log.info("arrêt")
    sd_notify("STOPPING=1")
    watcher.stop()
    await asyncio.to_thread(watcher.join, 5)
    await hub.close()
