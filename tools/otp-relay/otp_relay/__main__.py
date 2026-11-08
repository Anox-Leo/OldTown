"""Ligne de commande : run | check | tail | extract."""

from __future__ import annotations

import argparse
import asyncio
import imaplib
import json
import logging
import os
import socket
import ssl
import sys
from pathlib import Path

from . import __version__
from .config import DEFAULT_CONFIG_PATH, ConfigError, default_socket_path, load


def _logging(level: str) -> None:
    fmt = "%(levelname)s %(name)s: %(message)s"
    if not os.environ.get("JOURNAL_STREAM"):  # journald horodate déjà
        fmt = "%(asctime)s " + fmt
    logging.basicConfig(level=level, format=fmt, stream=sys.stderr)


def cmd_run(args: argparse.Namespace) -> int:
    from .daemon import run

    cfg = load(args.config)
    _logging(cfg.log_level)
    asyncio.run(run(cfg))
    return 0


def cmd_check(args: argparse.Namespace) -> int:
    from .imap_watcher import ImapWatcher, _quote

    cfg = load(args.config)
    print(f"configuration : {args.config}")
    print(f"socket        : {cfg.socket.path} (mode {cfg.socket.mode:o}, uid {cfg.socket.allowed_uids})")
    password = cfg.imap.password()
    watcher = ImapWatcher(cfg.imap, password, lambda *a: None, lambda *a: None)
    imap = watcher._connect()
    try:
        imap.login(cfg.imap.username, password)
        print(f"connexion     : OK ({cfg.imap.username} sur {cfg.imap.host})")
        print(f"IDLE          : {'oui' if 'IDLE' in imap.capabilities else 'NON'}")
        typ, data = imap.select(_quote(cfg.imap.mailbox), readonly=True)
        print(f"{cfg.imap.mailbox:<13} : {typ} {data[0].decode()} messages")
    finally:
        try:
            imap.logout()
        except Exception:
            pass
    return 0


def cmd_tail(args: argparse.Namespace) -> int:
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
        sock.connect(str(args.socket))
        for line in sock.makefile("r", encoding="utf-8"):
            msg = json.loads(line)
            if args.json:
                print(line, end="", flush=True)
            elif msg["type"] == "otp":
                flag = "✔" if msg["sender_verified"] else "⚠ non vérifié"
                replay = " (replay)" if msg.get("replay") else ""
                print(f"{msg['detected_at']}  {msg['code']:<10} {msg['sender']['domain']}  {flag}  "
                      f"confiance {msg['confidence']:.2f}  « {msg['subject']} »{replay}", flush=True)
            elif msg["type"] != "heartbeat":
                print(f"[{msg['type']}] " + json.dumps({k: v for k, v in msg.items() if k != "type"},
                                                       ensure_ascii=False), flush=True)
    return 0


def cmd_extract(args: argparse.Namespace) -> int:
    from .extract import detect, parse

    for path in args.files:
        mail = parse(Path(path).read_bytes())
        found = detect(mail)
        print(json.dumps({
            "file": str(path),
            "from": mail.from_address,
            "subject": mail.subject,
            "auth": mail.auth,
            "code": found.code if found else None,
            "display": found.display if found else None,
            "confidence": found.confidence if found else None,
        }, ensure_ascii=False))
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="otp-relay", description=__doc__)
    parser.add_argument("--version", action="version", version=f"otp-relay {__version__}")
    sub = parser.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("run", help="lance le daemon")
    p.add_argument("-c", "--config", type=Path, default=DEFAULT_CONFIG_PATH)
    p.set_defaults(func=cmd_run)

    p = sub.add_parser("check", help="vérifie la configuration et la connexion IMAP")
    p.add_argument("-c", "--config", type=Path, default=DEFAULT_CONFIG_PATH)
    p.set_defaults(func=cmd_check)

    p = sub.add_parser("tail", help="affiche les événements diffusés par le daemon")
    p.add_argument("-s", "--socket", type=Path, default=default_socket_path())
    p.add_argument("--json", action="store_true", help="JSON Lines bruts")
    p.set_defaults(func=cmd_tail)

    p = sub.add_parser("extract", help="teste l'extraction sur des fichiers .eml")
    p.add_argument("files", nargs="+", type=Path)
    p.set_defaults(func=cmd_extract)

    args = parser.parse_args(argv)
    try:
        return args.func(args)
    except ConfigError as exc:
        print(f"erreur de configuration : {exc}", file=sys.stderr)
        return 2
    except (imaplib.IMAP4.error, OSError, ssl.SSLError) as exc:
        print(f"erreur : {type(exc).__name__}: {exc}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    sys.exit(main())
