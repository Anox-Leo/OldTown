"""Détection d'un code à usage unique dans un e-mail brut (RFC 5322).

Heuristique à points : chaque suite de 4 à 8 caractères qui ressemble à un code
reçoit un score selon son format, sa proximité avec un mot-clé (« code »,
« verification », « OTP »…), sa mise en valeur dans le HTML, sa présence dans
le sujet, et des pénalités (année, montant, n° de commande, téléphone…).
"""

from __future__ import annotations

import email
import email.policy
import email.utils
import hashlib
import re
from dataclasses import dataclass, field
from email.message import EmailMessage
from html.parser import HTMLParser

MIN_SCORE = 5
MAX_SCORE = 10

KEYWORDS = re.compile(
    r"\b(?:codes?|otp|passcode|pin|token|"
    r"v[ée]rifications?|verify|v[ée]rifier|"
    r"one[\s-]time|single[\s-]use|usage\s+unique|"
    r"2fa|mfa|two[\s-](?:factor|step)|deux\s+[ée]tapes|"
    r"authentification|authentication|"
    r"login|sign[\s-]?in|connexion|se\s+connecter|"
    r"best[äa]tigungscode|sicherheitscode|anmeldecode|c[óo]digo|codice)\b",
    re.I,
)
# « code » dans ces expressions ne désigne pas un OTP : neutralisé avant la recherche de mots-clés.
NOT_OTP = re.compile(
    r"\b(?:(?:promo|discount|coupon|referral|gift|zip|postal|area|source|qr|bar)\s*codes?|"
    r"codes?\s+(?:promo|postal|de\s+(?:r[ée]duction|parrainage|remise)|cadeau|source|qr|barre))\b",
    re.I,
)
# Contexte immédiatement avant un nombre qui n'est pas un OTP.
NEGATIVE_BEFORE = re.compile(
    r"(?:promo|coupon|discount|r[ée]duction|remise|parrainage|referral|postal|zip|"
    r"commande|order|facture|invoice|n°|no\.|num[ée]ro|r[ée]f(?:[ée]rence)?|ref\.?|"
    r"tracking|suivi|colis|parcel|ticket|dossier|case|t[ée]l(?:[ée]phone)?|phone|"
    r"fax|siret|siren|iban|bic|client|customer|account\s+(?:number|no)|compte\s+n|"
    r"rue|street|avenue|bd|cedex|bp|[$€£]|eur|usd|total|prix|price|montant|amount)"
    r"[\s:#.\-]*$",
    re.I,
)
NEGATIVE_AFTER = re.compile(r"^\s*(?:€|eur\b|usd\b|\$|£|%|ko\b|mo\b|kb\b|mb\b|px\b|pts?\b|points?\b)", re.I)
MONTHS = re.compile(
    r"\b(?:janv?|f[ée]vr?|mars|avr|mai|juin|juil|ao[uû]t|sept?|oct|nov|d[ée]c|"
    r"jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-zéû]*\.?\s*$",
    re.I,
)
CANDIDATE = re.compile(
    r"(?<![\w@#/.,+=&?%$€£\-])"
    r"(?P<raw>"
    r"(?P<prefix>[A-Z]{1,3}-)?"
    r"(?:"
    r"(?P<grouped>\d{3,4}[ \-]\d{3,4})"
    r"|(?P<digits>\d{4,8})"
    r"|(?P<alnum>(?=[A-Z0-9\-]*\d)(?=[A-Z0-9\-]*[A-Z])[A-Z0-9]{3,4}-?[A-Z0-9]{2,4})"
    r"))"
    r"(?![\w@/%€$£\-]|[.,]\d)"
)
URLS = re.compile(r"(?:https?://|www\.)\S+|\S+@\S+\.[a-z]{2,}", re.I)
INVISIBLE = re.compile(r"[​-‏⁠﻿͏­]")


@dataclass
class ParsedMail:
    message_id: str
    subject: str
    from_name: str
    from_address: str
    from_domain: str
    to_address: str
    auth: dict[str, str]
    text: str
    emphasized: list[str] = field(default_factory=list)

    @property
    def event_id(self) -> str:
        return hashlib.sha256(self.message_id.encode()).hexdigest()[:16]


@dataclass
class Detection:
    code: str  # forme à saisir (sans espace, tiret ni préfixe G-)
    display: str  # forme telle qu'écrite dans le mail
    score: int
    confidence: float


# --------------------------------------------------------------------------- HTML


class _HtmlText(HTMLParser):
    BLOCK = {
        "p", "div", "td", "th", "tr", "table", "li", "ul", "ol", "h1", "h2", "h3", "h4",
        "h5", "h6", "section", "article", "header", "footer", "center", "blockquote", "pre",
    }
    VOID = {"br", "img", "hr", "meta", "link", "input", "wbr", "col", "area", "base", "source"}
    SKIP = {"script", "style", "head", "title", "noscript", "template"}
    EMPH = {"b", "strong", "h1", "h2", "h3", "code", "kbd", "tt", "pre", "mark", "big"}

    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.parts: list[str] = []
        self.emphasized: list[str] = []
        self._stack: list[tuple[str, str]] = []
        self._skip = 0
        self._emph = 0

    @staticmethod
    def _style_emphasis(style: str) -> bool:
        style = style.lower()
        if m := re.search(r"font-size\s*:\s*([\d.]+)\s*(px|pt|em|rem)", style):
            size, unit = float(m[1]), m[2]
            px = size * {"px": 1, "pt": 1.333, "em": 16, "rem": 16}[unit]
            if px >= 20:
                return True
        if re.search(r"letter-spacing\s*:\s*(?!0[;\s]|0$|normal)", style):
            return True
        if re.search(r"font-weight\s*:\s*(?:bold|[6-9]00)", style):
            return True
        return bool(re.search(r"font-family\s*:[^;]*(?:mono|courier|consolas)", style))

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if tag == "body":  # protège contre un <head> jamais fermé
            self._skip = 0
            self._stack.clear()
        if tag in self.BLOCK or tag == "br":
            self.parts.append("\n")
        if tag in self.VOID:
            return
        if tag in self.SKIP:
            self._skip += 1
            self._stack.append((tag, "skip"))
            return
        style = next((v for k, v in attrs if k == "style" and v), "")
        emph = tag in self.EMPH or self._style_emphasis(style)
        self._stack.append((tag, "emph" if emph else ""))
        if emph:
            self._emph += 1

    def handle_endtag(self, tag: str) -> None:
        for i in range(len(self._stack) - 1, -1, -1):
            if self._stack[i][0] == tag:
                for _, flag in self._stack[i:]:
                    if flag == "skip":
                        self._skip -= 1
                    elif flag == "emph":
                        self._emph -= 1
                del self._stack[i:]
                break
        if tag in self.BLOCK:
            self.parts.append("\n")

    def handle_data(self, data: str) -> None:
        if self._skip:
            return
        self.parts.append(data)
        if self._emph and data.strip():
            self.emphasized.append(data.strip())


def _normalize(text: str) -> str:
    text = INVISIBLE.sub("", text).replace("\xa0", " ").replace(" ", " ")
    lines = (re.sub(r"[ \t\r\f\v]+", " ", line).strip() for line in text.split("\n"))
    return re.sub(r"\n{3,}", "\n\n", "\n".join(lines)).strip()


def _html_to_text(html: str) -> tuple[str, list[str]]:
    parser = _HtmlText()
    try:
        parser.feed(html)
        parser.close()
    except Exception:  # HTML très cassé : on garde ce qui a été lu
        pass
    return _normalize("".join(parser.parts)), [_normalize(e) for e in parser.emphasized]


# --------------------------------------------------------------------------- parsing


def _content(part: EmailMessage) -> str:
    try:
        return part.get_content()
    except (LookupError, UnicodeError, AssertionError):
        payload = part.get_payload(decode=True) or b""
        return payload.decode(part.get_content_charset() or "utf-8", errors="replace")


def _authentication_results(msg: EmailMessage) -> dict[str, str]:
    """Verdicts SPF/DKIM/DMARC posés par Gmail.

    Seul l'en-tête dont l'authserv-id est mx.google.com fait foi : un expéditeur
    peut ajouter ses propres en-têtes Authentication-Results plus bas.
    """
    for header in msg.get_all("Authentication-Results", []):
        value = str(header)
        if value.strip().lower().startswith("mx.google.com"):
            return {k.lower(): v.lower() for k, v in re.findall(r"\b(spf|dkim|dmarc)=(\w+)", value, re.I)}
    return {}


def parse(raw: bytes) -> ParsedMail:
    msg = email.message_from_bytes(raw, policy=email.policy.default)
    assert isinstance(msg, EmailMessage)

    from_name, from_address = email.utils.parseaddr(str(msg.get("From", "")))
    to_header = msg.get("Delivered-To") or msg.get("To") or ""
    _, to_address = email.utils.parseaddr(str(to_header))
    message_id = str(msg.get("Message-ID") or "").strip()
    if not message_id:
        message_id = "sha256:" + hashlib.sha256(raw).hexdigest()

    html_part = msg.get_body(preferencelist=("html",))
    plain_part = msg.get_body(preferencelist=("plain",))
    emphasized: list[str] = []
    if html_part is not None:
        text, emphasized = _html_to_text(_content(html_part))
        if not text and plain_part is not None:
            text = _normalize(_content(plain_part))
    elif plain_part is not None:
        text = _normalize(_content(plain_part))
    else:
        text = ""

    return ParsedMail(
        message_id=message_id,
        subject=_normalize(str(msg.get("Subject", ""))),
        from_name=from_name,
        from_address=from_address.lower(),
        from_domain=from_address.rpartition("@")[2].lower(),
        to_address=to_address.lower(),
        auth=_authentication_results(msg),
        text=text,
        emphasized=emphasized,
    )


# --------------------------------------------------------------------------- scoring


def _canonical(m: re.Match[str]) -> str:
    body = m["grouped"] or m["digits"] or m["alnum"]
    return re.sub(r"[ \-]", "", body)


def _score(m: re.Match[str], text: str, *, in_subject: bool, emphasized: bool) -> int:
    start, end = m.span("raw")
    code = _canonical(m)
    before = text[max(0, start - 80):start]
    near_before = text[max(0, start - 40):start]
    after = text[end:end + 50]
    line_start = text.rfind("\n", 0, start) + 1
    line_end = text.find("\n", end)
    line = text[line_start:line_end if line_end != -1 else len(text)].strip(" .:!")

    score = 0
    if m["alnum"]:
        score += 0
    elif len(code) == 6:
        score += 3
    elif len(code) in (5, 7, 8):
        score += 2
    else:
        score += 1
    if m["prefix"] and not m["alnum"]:
        score += 2  # ex. « G-123456 » chez Google

    if KEYWORDS.search(near_before):
        score += 5
    elif KEYWORDS.search(before):
        score += 4
    elif KEYWORDS.search(after):
        score += 3

    if line == m["raw"]:
        score += 3
    if emphasized:
        score += 2
    if in_subject:
        score += 2

    if NEGATIVE_BEFORE.search(text[max(0, start - 30):start]) or NEGATIVE_AFTER.search(after):
        score -= 6
    if m["digits"] and len(code) == 4 and 1900 <= int(code) <= 2099:
        score -= 4
    if MONTHS.search(text[max(0, start - 12):start]):
        score -= 4
    return score


def _mask(pattern: re.Pattern[str], text: str) -> str:
    """Remplace les correspondances par des espaces en conservant les positions."""
    return pattern.sub(lambda m: " " * len(m[0]), text)


def _candidates(text: str) -> list[re.Match[str]]:
    return list(CANDIDATE.finditer(_mask(URLS, text)))


def detect(mail: ParsedMail) -> Detection | None:
    corpus = _mask(NOT_OTP, f"{mail.subject}\n\n{mail.text}")
    if not KEYWORDS.search(corpus):
        return None

    subject_codes = {_canonical(m) for m in _candidates(mail.subject)}
    emphasized_codes = {_canonical(m) for chunk in mail.emphasized if len(chunk) <= 40 for m in _candidates(chunk)}

    best: dict[str, tuple[int, str]] = {}
    clean = _mask(URLS, corpus)
    for m in CANDIDATE.finditer(clean):
        code = _canonical(m)
        score = _score(m, clean, in_subject=code in subject_codes, emphasized=code in emphasized_codes)
        if code not in best or score > best[code][0]:
            best[code] = (score, m["raw"])
    if not best:
        return None

    ranked = sorted(best.items(), key=lambda kv: kv[1][0], reverse=True)
    code, (score, display) = ranked[0]
    if score < MIN_SCORE:
        return None
    if len(ranked) > 1 and ranked[1][1][0] >= score - 1:
        score -= 2  # deux candidats presque à égalité : on baisse la confiance
    return Detection(code=code, display=display, score=score, confidence=round(min(1.0, max(0.0, score / MAX_SCORE)), 2))
