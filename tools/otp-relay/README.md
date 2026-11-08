# otp-relay

A daemon that keeps an IMAP IDLE connection open on a mailbox, detects
one-time codes (OTP) in incoming mail and publishes them as JSON Lines on a
local Unix socket. `ot otp-bridge` reads that socket and sends each code,
end-to-end encrypted, to your OldTown devices; the browser extension can
then fill it in the page.

```
Mailbox ──IMAP IDLE (TLS)──▶ otp-relay ──Unix socket, JSON Lines──▶ ot otp-bridge ──▶ devices
```

Python ≥ 3.14, standard library only. Nothing but the code, its sender's
name and domain, the DMARC verdict and the expiry leaves the machine: the
bridge drops the subject, addresses and headers.

The socket contract is described in [PROTOCOL.md](PROTOCOL.md).

## Setup

1. **App password.** With Gmail: enable 2-step verification, then create an
   app password at <https://myaccount.google.com/apppasswords>. If Gmail
   still shows an IMAP switch (Settings → Forwarding and POP/IMAP), turn it
   on.
2. **Install** (no root): `./install.sh` copies the code to
   `~/.local/lib/otp-relay`, the command to `~/.local/bin/otp-relay`, an
   example configuration to `~/.config/otp-relay/config.toml` (only if
   absent) and the systemd user unit (not enabled).
3. **Configure** `~/.config/otp-relay/config.toml` (at least
   `imap.username`), then:
   ```sh
   install -m 600 /dev/null ~/.config/otp-relay/imap-password
   $EDITOR ~/.config/otp-relay/imap-password   # paste the app password
   otp-relay check                              # tests the connection
   ```
4. **Start at boot, even without a session:**
   ```sh
   sudo loginctl enable-linger "$USER"
   systemctl --user enable --now otp-relay
   otp-relay tail                               # shows codes live
   ```
5. **Bridge to OldTown:** see `deploy/ot-otp-bridge.service` at the root of
   the repository (`ot otp-bridge --to <device>`).

## Commands

| Command | Purpose |
| --- | --- |
| `otp-relay run [-c config.toml]` | runs the daemon (what the systemd unit does) |
| `otp-relay check` | checks the configuration, password, connection and IDLE support |
| `otp-relay tail [--json]` | debug client: prints socket events |
| `otp-relay extract mail.eml…` | tests detection on exported mails (Gmail → "Download message") |

Logs: `journalctl --user -u otp-relay -f`. Codes are masked (`48••••`)
unless `log.show_codes = true`.

## How it works

- `SELECT` read-only, then `IDLE`. As soon as the server reports
  `* N EXISTS`, the daemon leaves IDLE, fetches the new messages with
  `UID FETCH … BODY.PEEK[]` (without marking them read) and goes back to
  IDLE. The daemon adds under 50 ms to the server's delay.
- IDLE is renewed every `idle_refresh` seconds (10 min by default; Gmail
  drops it after about 29 min), and each renewal searches for new mail.
  Gmail sometimes reports a mail late in IDLE (30 s or more):
  `idle_refresh = 10` bounds that delay to about ten seconds, at the cost of
  a small request every 10 s. TCP keepalive detects a dead connection in
  about 2 min. Reconnection uses exponential backoff (1 s → 5 min) and
  catches up on missed UIDs.
- After an authentication failure, the daemon waits 15 min before retrying,
  so as not to get the account locked.
- Only mails received less than `imap.max_age` ago (15 min) are published.
- Detection: a heuristic score (format, proximity of an FR/EN/DE/ES/IT
  keyword, HTML emphasis, presence in the subject, penalties for years,
  amounts, order numbers…).
- `sender_verified` is the DMARC verdict, read **only** from the
  `Authentication-Results` header whose authserv-id is `mx.google.com`: a
  sender can add its own such headers further down. With another provider,
  no code is ever verified, so the extension never fills one by itself (the
  keyboard shortcut still works).

## Tests

```sh
python3 -m unittest discover -s tests -t .

# End to end, against a throwaway IMAP server:
docker run -d --name otp-relay-it -p 127.0.0.1:11143:143 \
  -v "$PWD/tests/dovecot-plain.conf:/etc/dovecot/conf.d/zz-plain.conf:ro" dovecot/dovecot:2.3-latest
OTP_RELAY_IT=127.0.0.1:11143:test:pass OTP_RELAY_IT_DOCKER=otp-relay-it \
  python3 -m unittest tests.test_integration -v
docker rm -f otp-relay-it
```
