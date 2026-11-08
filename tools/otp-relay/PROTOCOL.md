# otp-relay socket protocol (version 1)

The interface between otp-relay and its clients, such as `ot otp-bridge`.
An incompatible change bumps `protocol` in `hello`; a client must refuse a
version it does not know.

## Transport and access

- Unix stream socket, by default `$XDG_RUNTIME_DIR/otp-relay/otp-relay.sock`
  (`socket.path`). The directory is `0700`, the socket `0600`
  (`socket.mode`; never any right for "others").
- The peer's UID is checked with `SO_PEERCRED` against `socket.allowed_uids`
  (default: the daemon's own user). A refused client gets
  `{"type":"error","error":"forbidden"}` and is disconnected; so is any
  client beyond `socket.max_clients` (32).
- JSON Lines, UTF-8, one object per line, at most 64 KiB per line.
- Timestamps are ISO 8601 in UTC with milliseconds
  (`2026-11-02T14:03:07.120Z`).
- A client that does not keep up (256 pending messages) is disconnected.

## Connection sequence

On each connection the daemon sends, in this order:

1. `hello`;
2. the current `status`;
3. every code still valid, with `"replay": true`;
4. then live events, and a `heartbeat` every `socket.heartbeat` seconds
   (30). A client that hears nothing for about 2.5 heartbeats should assume
   the daemon is gone and reconnect.

## Daemon → client

| `type` | Fields | Meaning |
| --- | --- | --- |
| `hello` | `protocol` (1), `daemon` (`otp-relay/0.1.0`), `code_ttl` (s), `heartbeat` (s) | First line of every connection |
| `status` | `state`, `detail`, `since` | IMAP state: `starting`, `connected`, `disconnected`, `auth_failed`, `stopped` |
| `otp` | see below | A detected code |
| `expired` | `id` | A code reached its expiry |
| `consumed` | `id`, `by`, `at` | A client marked the code as used |
| `heartbeat` | `at`, `imap` (current `state`) | Liveness |
| `pong`, `list`, `ack`, `error` | see "Commands" | Replies |

### `otp`

```json
{
  "type": "otp",
  "id": "3f1c9a0b7d2e4f61",
  "code": "583920",
  "display": "G-583920",
  "confidence": 0.9,
  "sender": {"name": "Google", "address": "no-reply@accounts.google.com", "domain": "accounts.google.com"},
  "sender_verified": true,
  "auth": {"spf": "pass", "dkim": "pass", "dmarc": "pass"},
  "to": "me@example.org",
  "subject": "G-583920 is your Google verification code",
  "received_at": "2026-11-02T14:03:05.000Z",
  "detected_at": "2026-11-02T14:03:05.210Z",
  "expires_at": "2026-11-02T14:13:05.000Z",
  "source": {"mailbox": "INBOX", "uid": 48213}
}
```

- `id`: 16 hex characters, derived from the `Message-ID`. While a code is
  valid, the daemon never publishes the same `id` twice, even if the server
  reports the mail again. A client should still deduplicate by `id`: every
  connection replays the codes still valid, and a restarted daemon may
  publish a recent mail again.
- `code`: what to type (no space, dash or `G-` prefix). `display`: the code
  as written in the mail.
- `confidence`: between 0 and 1; codes under `events.min_confidence` are
  never published.
- `sender_verified`: `auth.dmarc == "pass"`, as seen by Gmail
  (`Authentication-Results` from `mx.google.com` only); `false` otherwise.
- `expires_at`: reception + `events.code_ttl` (600 s), and at least one
  minute after detection.
- `replay`: present and `true` only for codes sent at connection time.

## Commands (client → daemon)

One JSON object per line with a `cmd` field. An optional `ref` (any JSON
value) is echoed in the reply.

| `cmd` | Extra fields | Reply |
| --- | --- | --- |
| `ping` | | `{"type":"pong"}` |
| `list` | | `{"type":"list","events":[…],"status":{…}}`: codes still valid, as on replay |
| `consume` | `id`, optional `by` (≤ 64 chars) | `{"type":"ack","ok":true}` if the code was active, then `consumed` to every client |

Errors: `{"type":"error","error":…}` with `bad_request` (not JSON, or no
`cmd`), `unknown_command`, `line_too_long` (the connection is then closed)
or `forbidden`.

See `contrib/example_client.py` for a minimal client with reconnection,
silence detection and deduplication.
