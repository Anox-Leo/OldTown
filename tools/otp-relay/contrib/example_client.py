"""Client minimal d'otp-relay : reconnexion, détection de daemon muet, dédoublonnage."""

import asyncio
import json
import os
import time

SOCKET = f"/run/user/{os.getuid()}/otp-relay/otp-relay.sock"
SILENCE_LIMIT = 75  # 2,5 heartbeats


async def handle(event: dict, seen: set[str], writer: asyncio.StreamWriter) -> None:
    if event["id"] in seen:
        return
    seen.add(event["id"])
    # À remplacer par l'envoi chiffré vers les appareils.
    print(f"{event['code']} de {event['sender']['domain']} (vérifié : {event['sender_verified']})")
    # Une fois le code utilisé sur un appareil :
    # writer.write(json.dumps({"cmd": "consume", "id": event["id"], "by": "pixel-8"}).encode() + b"\n")


async def session(seen: set[str]) -> None:
    reader, writer = await asyncio.open_unix_connection(SOCKET)
    try:
        while True:
            line = await asyncio.wait_for(reader.readline(), SILENCE_LIMIT)
            if not line:
                raise ConnectionError("socket fermé par le daemon")
            msg = json.loads(line)
            match msg.get("type"):
                case "hello" if msg["protocol"] != 1:
                    raise RuntimeError(f"protocole {msg['protocol']} non supporté")
                case "otp":
                    await handle(msg, seen, writer)
                case "status":
                    print(f"IMAP : {msg['state']} {msg['detail']}")
                case "consumed" | "expired":
                    pass  # retirer le code des appareils
                case "error":
                    raise ConnectionError(msg["error"])
    finally:
        writer.close()


async def main() -> None:
    seen: set[str] = set()
    backoff = 1
    while True:
        started = time.monotonic()
        try:
            await session(seen)
        except (OSError, ConnectionError, TimeoutError) as exc:
            print(f"déconnecté : {exc!r}")
        if time.monotonic() - started > 60:
            backoff = 1
        await asyncio.sleep(backoff)
        backoff = min(backoff * 2, 30)


if __name__ == "__main__":
    asyncio.run(main())
