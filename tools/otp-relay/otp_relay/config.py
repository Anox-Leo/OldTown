"""Chargement et validation de la configuration (TOML)."""

from __future__ import annotations

import ipaddress
import os
import socket
import stat
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

CONFIG_HOME = Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config")
DEFAULT_CONFIG_PATH = CONFIG_HOME / "otp-relay" / "config.toml"
PASSWORD_ENV = "OTP_RELAY_IMAP_PASSWORD"


class ConfigError(Exception):
    pass


def default_socket_path() -> Path:
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    if runtime:
        return Path(runtime) / "otp-relay" / "otp-relay.sock"
    return Path(f"/tmp/otp-relay-{os.getuid()}") / "otp-relay.sock"


@dataclass
class ImapConfig:
    username: str
    password_file: Path | None = None
    host: str = "imap.gmail.com"
    port: int = 993
    security: str = "ssl"  # ssl | starttls | none (boucle locale uniquement, pour les tests)
    mailbox: str = "INBOX"
    # Renouvellement de l'IDLE, qui refait aussi une recherche des nouveaux mails
    # (Gmail tarde parfois à signaler un mail) ; Gmail coupe un IDLE vers 29 min.
    idle_refresh: int = 600
    max_age: int = 900  # un mail plus vieux que ça n'est jamais diffusé
    connect_timeout: int = 30

    def password(self) -> str:
        env = os.environ.get(PASSWORD_ENV)
        if env:
            return env.replace(" ", "")
        if self.password_file is None:
            raise ConfigError(f"ni imap.password_file ni ${PASSWORD_ENV} ne sont définis")
        path = self.password_file
        try:
            st = path.stat()
        except FileNotFoundError:
            raise ConfigError(f"fichier de mot de passe introuvable : {path}") from None
        if st.st_mode & (stat.S_IRWXG | stat.S_IRWXO):
            raise ConfigError(f"{path} est lisible par d'autres utilisateurs : chmod 600 {path}")
        # Les mots de passe d'application Google s'affichent par groupes de 4 séparés d'espaces.
        value = path.read_text().strip().replace(" ", "")
        if not value:
            raise ConfigError(f"{path} est vide")
        return value


@dataclass
class SocketConfig:
    path: Path = field(default_factory=default_socket_path)
    mode: int = 0o600
    allowed_uids: list[int] = field(default_factory=lambda: [os.getuid()])
    heartbeat: int = 30
    max_clients: int = 32


@dataclass
class Config:
    imap: ImapConfig
    socket: SocketConfig = field(default_factory=SocketConfig)
    code_ttl: int = 600
    min_confidence: float = 0.4
    log_level: str = "INFO"
    log_codes: bool = False


def _path(value: str) -> Path:
    return Path(os.path.expandvars(value)).expanduser()


def _take(section: dict, name: str, allowed: set[str]) -> None:
    unknown = set(section) - allowed
    if unknown:
        raise ConfigError(f"clé(s) inconnue(s) dans [{name}] : {', '.join(sorted(unknown))}")


def _is_loopback(host: str) -> bool:
    try:
        return ipaddress.ip_address(socket.gethostbyname(host)).is_loopback
    except (OSError, ValueError):
        return False


def load(path: Path = DEFAULT_CONFIG_PATH) -> Config:
    try:
        with open(path, "rb") as fh:
            raw = tomllib.load(fh)
    except FileNotFoundError:
        raise ConfigError(f"configuration introuvable : {path}") from None
    except tomllib.TOMLDecodeError as exc:
        raise ConfigError(f"{path} : TOML invalide : {exc}") from None

    _take(raw, "racine", {"imap", "socket", "events", "log"})

    imap_raw = dict(raw.get("imap", {}))
    _take(imap_raw, "imap", {f for f in ImapConfig.__dataclass_fields__})
    if not imap_raw.get("username"):
        raise ConfigError("imap.username est obligatoire")
    if "password_file" in imap_raw:
        imap_raw["password_file"] = _path(imap_raw["password_file"])
    imap = ImapConfig(**imap_raw)
    if imap.security not in {"ssl", "starttls", "none"}:
        raise ConfigError("imap.security doit valoir ssl, starttls ou none")
    if imap.security == "none" and not _is_loopback(imap.host):
        raise ConfigError("imap.security = none n'est autorisé que vers 127.0.0.1 (tests)")
    # Bas : Gmail tarde parfois à signaler un mail en IDLE ; chaque renouvellement
    # refait un UID SEARCH, qui le trouve sans attendre ce signal.
    if not 5 <= imap.idle_refresh <= 1500:
        raise ConfigError("imap.idle_refresh doit être compris entre 5 et 1500 secondes")

    sock_raw = dict(raw.get("socket", {}))
    _take(sock_raw, "socket", {f for f in SocketConfig.__dataclass_fields__})
    if "path" in sock_raw:
        sock_raw["path"] = _path(sock_raw["path"])
    if isinstance(sock_raw.get("mode"), str):
        sock_raw["mode"] = int(sock_raw["mode"], 8)
    sock = SocketConfig(**sock_raw)
    if sock.mode & 0o007:
        raise ConfigError("socket.mode ne doit donner aucun droit aux « autres » (ex. 0600 ou 0660)")

    events = raw.get("events", {})
    _take(events, "events", {"code_ttl", "min_confidence"})
    log = raw.get("log", {})
    _take(log, "log", {"level", "show_codes"})

    return Config(
        imap=imap,
        socket=sock,
        code_ttl=int(events.get("code_ttl", 600)),
        min_confidence=float(events.get("min_confidence", 0.4)),
        log_level=str(log.get("level", "INFO")).upper(),
        log_codes=bool(log.get("show_codes", False)),
    )
