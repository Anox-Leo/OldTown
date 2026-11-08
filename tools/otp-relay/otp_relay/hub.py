"""Diffusion des événements sur un socket Unix, protocole JSON Lines.

Chaque client connecté reçoit : `hello`, l'état IMAP courant, les codes encore
valides (`replay: true`), puis les événements en direct et un `heartbeat`
périodique. Un client peut envoyer `ping`, `list` et `consume`.
Le contrat complet est décrit dans PROTOCOL.md.
"""

from __future__ import annotations

import asyncio
import json
import logging
import os
import socket
import stat
import struct
import time
from pathlib import Path
from typing import Any

from . import PROTOCOL_VERSION, __version__
from .config import SocketConfig

log = logging.getLogger(__name__)

CLIENT_QUEUE = 256
MAX_LINE = 64 * 1024


def iso(epoch: float) -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(epoch)) + f".{int(epoch % 1 * 1000):03d}Z"


class _Client:
    def __init__(self, writer: asyncio.StreamWriter, uid: int, pid: int) -> None:
        self.writer = writer
        self.uid = uid
        self.pid = pid
        self.queue: asyncio.Queue[bytes | None] = asyncio.Queue(CLIENT_QUEUE)

    def send(self, message: dict[str, Any]) -> bool:
        try:
            self.queue.put_nowait(json.dumps(message, ensure_ascii=False).encode() + b"\n")
            return True
        except asyncio.QueueFull:
            return False


class Hub:
    def __init__(self, cfg: SocketConfig, code_ttl: int) -> None:
        self.cfg = cfg
        self.code_ttl = code_ttl
        self.active: dict[str, tuple[float, dict[str, Any]]] = {}  # id → (expiration epoch, événement)
        self.status: dict[str, Any] = {"type": "status", "state": "starting", "detail": "", "since": iso(time.time())}
        self._clients: set[_Client] = set()
        self._server: asyncio.AbstractServer | None = None
        self._heartbeat: asyncio.Task | None = None

    # ------------------------------------------------------------------ serveur

    async def start(self) -> None:
        path = self.cfg.path
        _prepare_dir(path.parent)
        if path.exists() or path.is_symlink():
            if not stat.S_ISSOCK(path.lstat().st_mode):
                raise RuntimeError(f"{path} existe et n'est pas un socket")
            path.unlink()  # socket orphelin d'une exécution précédente
        old_umask = os.umask(0o177)
        try:
            self._server = await asyncio.start_unix_server(self._handle, path=str(path), limit=MAX_LINE)
        finally:
            os.umask(old_umask)
        os.chmod(path, self.cfg.mode)
        self._heartbeat = asyncio.create_task(self._heartbeat_loop())
        log.info("socket prêt : %s (mode %o, uid autorisés %s)", path, self.cfg.mode, self.cfg.allowed_uids)

    async def close(self) -> None:
        if self._heartbeat:
            self._heartbeat.cancel()
        for client in list(self._clients):
            self._drop(client)
        if self._server:
            self._server.close()
            await self._server.wait_closed()
        try:
            self.cfg.path.unlink()
        except FileNotFoundError:
            pass

    # ------------------------------------------------------------------ publication

    def publish(self, event: dict[str, Any], expires_epoch: float) -> None:
        self._expire()
        if event["id"] in self.active or expires_epoch <= time.time():
            return  # déjà diffusé (message re-signalé après reconnexion) ou périmé
        self.active[event["id"]] = (expires_epoch, event)
        self._broadcast(event)

    def set_status(self, state: str, detail: str) -> None:
        if state == self.status["state"] and detail == self.status["detail"]:
            return
        self.status = {"type": "status", "state": state, "detail": detail, "since": iso(time.time())}
        self._broadcast(self.status)

    def consume(self, event_id: str, by: str) -> bool:
        if self.active.pop(event_id, None) is None:
            return False
        self._broadcast({"type": "consumed", "id": event_id, "by": by, "at": iso(time.time())})
        return True

    def _expire(self) -> None:
        now = time.time()
        for event_id in [k for k, (exp, _) in self.active.items() if exp <= now]:
            del self.active[event_id]
            self._broadcast({"type": "expired", "id": event_id})

    def _snapshot(self) -> list[dict[str, Any]]:
        self._expire()
        return [dict(e, replay=True) for _, e in self.active.values()]

    def _broadcast(self, message: dict[str, Any]) -> None:
        for client in list(self._clients):
            if not client.send(message):
                log.warning("client pid=%d trop lent, déconnecté", client.pid)
                self._drop(client)

    def _drop(self, client: _Client) -> None:
        self._clients.discard(client)
        client.writer.close()

    async def _heartbeat_loop(self) -> None:
        while True:
            await asyncio.sleep(self.cfg.heartbeat)
            self._expire()
            self._broadcast({"type": "heartbeat", "at": iso(time.time()), "imap": self.status["state"]})

    # ------------------------------------------------------------------ clients

    async def _handle(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        sock: socket.socket = writer.get_extra_info("socket")
        pid, uid, _gid = struct.unpack("3i", sock.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize("3i")))
        if uid not in self.cfg.allowed_uids or len(self._clients) >= self.cfg.max_clients:
            log.warning("connexion refusée : pid=%d uid=%d", pid, uid)
            writer.write(json.dumps({"type": "error", "error": "forbidden"}).encode() + b"\n")
            writer.close()
            return

        client = _Client(writer, uid, pid)
        replay = self._snapshot()  # avant l'ajout : le client ne reçoit pas les « expired » de ce nettoyage
        client.send({"type": "hello", "protocol": PROTOCOL_VERSION, "daemon": f"otp-relay/{__version__}",
                     "code_ttl": self.code_ttl, "heartbeat": self.cfg.heartbeat})
        client.send(self.status)
        for event in replay:
            client.send(event)
        self._clients.add(client)
        log.info("client connecté : pid=%d uid=%d", pid, uid)

        writer_task = asyncio.create_task(self._pump(client))
        try:
            while not reader.at_eof():
                try:
                    line = await reader.readline()
                except (ValueError, asyncio.LimitOverrunError):
                    client.send({"type": "error", "error": "line_too_long"})
                    break
                if not line:
                    break
                self._command(client, line)
        except ConnectionError:
            pass
        finally:
            self._clients.discard(client)
            writer_task.cancel()
            writer.close()
            log.info("client déconnecté : pid=%d", pid)

    async def _pump(self, client: _Client) -> None:
        try:
            while (data := await client.queue.get()) is not None:
                client.writer.write(data)
                await client.writer.drain()
        except (ConnectionError, asyncio.CancelledError):
            pass
        finally:
            client.writer.close()

    def _command(self, client: _Client, line: bytes) -> None:
        try:
            msg = json.loads(line)
            cmd = msg["cmd"]
        except (ValueError, KeyError, TypeError):
            client.send({"type": "error", "error": "bad_request"})
            return
        ref = msg.get("ref")
        reply: dict[str, Any]
        match cmd:
            case "ping":
                reply = {"type": "pong"}
            case "list":
                reply = {"type": "list", "events": self._snapshot(), "status": self.status}
            case "consume" if isinstance(msg.get("id"), str):
                by = str(msg.get("by", f"pid:{client.pid}"))[:64]
                reply = {"type": "ack", "ok": self.consume(msg["id"], by)}
            case _:
                reply = {"type": "error", "error": "unknown_command"}
        if ref is not None:
            reply["ref"] = ref
        client.send(reply)


def _prepare_dir(directory: Path) -> None:
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    st = directory.lstat()
    if not stat.S_ISDIR(st.st_mode) or st.st_uid != os.getuid():
        raise RuntimeError(f"{directory} doit être un dossier appartenant à l'utilisateur courant")
    if st.st_mode & 0o002:
        raise RuntimeError(f"{directory} est inscriptible par tous : refusé")
