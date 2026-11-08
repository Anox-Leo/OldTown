import asyncio
import json
import os
import tempfile
import time
import unittest
from pathlib import Path

from otp_relay.config import SocketConfig
from otp_relay.hub import Hub


def event(event_id: str = "e1", code: str = "123456") -> dict:
    return {"type": "otp", "id": event_id, "code": code}


class HubTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.path = Path(self.tmp.name) / "run" / "otp.sock"
        self.hub = Hub(SocketConfig(path=self.path, heartbeat=3600), code_ttl=600)
        await self.hub.start()

    async def asyncTearDown(self):
        await self.hub.close()
        self.tmp.cleanup()

    async def connect(self):
        reader, writer = await asyncio.open_unix_connection(str(self.path))
        self.addCleanup(writer.close)
        return reader, writer

    async def recv(self, reader) -> dict:
        return json.loads(await asyncio.wait_for(reader.readline(), 2))

    async def test_socket_permissions(self):
        self.assertEqual(self.path.stat().st_mode & 0o777, 0o600)
        self.assertEqual(self.path.parent.stat().st_mode & 0o777, 0o700)

    async def test_hello_status_then_live_event(self):
        reader, _ = await self.connect()
        hello = await self.recv(reader)
        self.assertEqual(hello["type"], "hello")
        self.assertEqual(hello["protocol"], 1)
        self.assertEqual((await self.recv(reader))["type"], "status")
        self.hub.publish(event(), time.time() + 60)
        self.assertEqual((await self.recv(reader))["code"], "123456")

    async def test_replay_and_dedup(self):
        self.hub.publish(event(), time.time() + 60)
        self.hub.publish(event(), time.time() + 60)  # même id : ignoré
        self.hub.publish(event("old"), time.time() - 1)  # déjà expiré
        reader, _ = await self.connect()
        await self.recv(reader), await self.recv(reader)
        replayed = await self.recv(reader)
        self.assertEqual((replayed["id"], replayed["replay"]), ("e1", True))
        writer_cmd = (await self.connect())[1]
        writer_cmd.write(b'{"cmd":"ping","ref":7}\n')
        with self.assertRaises(asyncio.TimeoutError):
            await asyncio.wait_for(reader.readline(), 0.2)  # aucun autre replay

    async def test_consume_is_broadcast(self):
        self.hub.publish(event(), time.time() + 60)
        r1, _ = await self.connect()
        r2, w2 = await self.connect()
        for r in (r1, r2):
            for _ in range(3):
                await self.recv(r)
        w2.write(b'{"cmd":"consume","id":"e1","by":"phone","ref":"x"}\n')
        msgs = [await self.recv(r2), await self.recv(r2)]
        self.assertIn({"type": "ack", "ok": True, "ref": "x"}, msgs)
        consumed = await self.recv(r1)
        self.assertEqual((consumed["type"], consumed["by"]), ("consumed", "phone"))
        self.assertEqual(self.hub.active, {})

    async def test_commands(self):
        reader, writer = await self.connect()
        await self.recv(reader), await self.recv(reader)
        writer.write(b'{"cmd":"ping","ref":1}\nnot json\n{"cmd":"nope"}\n{"cmd":"list"}\n')
        self.assertEqual(await self.recv(reader), {"type": "pong", "ref": 1})
        self.assertEqual((await self.recv(reader))["error"], "bad_request")
        self.assertEqual((await self.recv(reader))["error"], "unknown_command")
        listing = await self.recv(reader)
        self.assertEqual((listing["type"], listing["events"]), ("list", []))

    async def test_forbidden_uid(self):
        self.hub.cfg.allowed_uids = [os.getuid() + 12345]
        reader, _ = await self.connect()
        self.assertEqual(await self.recv(reader), {"type": "error", "error": "forbidden"})
        self.assertEqual(await reader.read(), b"")


if __name__ == "__main__":
    unittest.main()
