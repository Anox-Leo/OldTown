# Security

OldTown encrypts content end to end: the relay only stores and forwards
ciphertext. It is a personal project published as is: the cryptography was
reviewed by its author and by an AI model, not yet by an independent
cryptographer. Use it at your own risk, and keep that in mind before
exposing a relay to the Internet or sending highly sensitive data.

## Reporting a vulnerability

Please use GitHub's private reporting (*Security* tab → *Report a
vulnerability*), not a public issue.

## Threat model

| Who | What they can do |
| --- | --- |
| The network between a device and the relay | Nothing useful: TLS (`wss`, HTTPS), and the content is end-to-end encrypted anyway. A TLS-intercepting proxy sees frames and metadata, never content. |
| The relay, or whoever reads its database | See metadata: device names and public keys, who writes to whom, when, sizes (files to the nearest 1 MiB chunk). Not read, alter, replay under another id, redirect or forge a message (associated data, member signatures). For files: neither name, type nor content; an altered, moved, removed or added chunk is rejected. |
| A malicious relay during pairing | Refuse the pairing, not hijack it: without the one-time secret of the code or QR (128 bits) it can neither read the request nor forge the answer. |
| A relay adding a device of its own | Make it appear in `ot devices` (flagged "outside the group"); it receives nothing readable and its messages are rejected. |
| Anyone who can reach the relay | Open at most 16 unauthenticated connections per IP (256 in total), 10 s each; fail authentication, try pairing tokens (80 bits, 10 minutes, single use) or sign file requests badly 30 times in a row, then once every 10 s, before being refused. |
| A device of my group | Read and write like my other devices, add a device, rotate the group key: a group serves one person. |
| Another person on the same relay | Read and write in their own group; write to the people they are linked to and see their names, reachable devices and presence. Not read my group, invite, link or see anyone else: the relay refuses (`forbidden`, `not_allowed`). |
| A service | Write one way to the people it is linked to; anyone can mute it. It has no group key and reads nothing. |
| The admin, who also hosts the relay | See everyone's metadata (who writes to whom, when, sizes), certify people, set links, revoke. Not read what other people write, unless they send it to the admin. Whoever runs a code bridge for someone reads that person's mailbox. |
| A relay showing a false directory | Hide a directory update (a removed device keeps receiving while the relay delivers to it), or deliver a message from a certified person who is not linked. Not invent a person or a device, replace a certificate, shorten or fork a roster log: clients check every certificate and log entry against the admin's anchor, and report a fork of their own roster. |
| Whoever intercepts an invitation | Take the invited person's place if they use it first, within 48 h; the admin sees the device name and a fingerprint to compare. The secret in the link (128 bits) never reaches the relay; the token is single use. |
| A stolen device | Everything its person can do, until it is removed (group key rotated without it, then revoked on the relay). A stolen admin device can also invite people and change links until it is removed; the certificates it signed are then re-signed by another admin device or lapse. Android keys are encrypted by an Android Keystore key and excluded from backups, but usable while the phone is locked. Extension keys are encrypted by a non-extractable WebCrypto key of the same browser profile: whoever opens the computer session can use them. |

## In place

- End-to-end encryption (`crates/ot-crypto`, RustCrypto primitives only):
  XChaCha20-Poly1305 under a group key per epoch; key transport with X25519
  and HKDF-SHA256; Ed25519 signature of every message; associated data
  covering header, `msg_id`, sender, recipients and creation time. Format
  frozen by test vectors cross-checked against an independent
  implementation.
- Clients reject plaintext, senders outside the roster and bad signatures;
  trust comes from pairing, never from the relay's device list.
- Several people: one group key per person. Messages to another person use
  a multi-recipient scheme (`0x05`): a message key sealed to each recipient
  device with X25519 and HKDF-SHA256 (bound to the device and `msg_id`), a
  key commitment so every recipient reads the same plaintext, the same
  Ed25519 signature as within a group. Test vectors cross-checked against an
  independent implementation.
- Signed directory: the admin's self-signed certificate is the anchor,
  carried by every invitation; each person has a certificate signed by an
  admin device and an append-only roster log signed by their own devices;
  the admin's log commits to every current certificate, so a replaced or
  revoked one cannot be replayed. Clients verify the whole chain, ignore
  devices outside a verified roster, and refuse a shorter or diverging admin
  log than the one they were given when they joined.
- Relay rules per person: delivery only within a person or along a link;
  admin frames only from admin devices; per-person quotas (devices, storage,
  send rate, pending messages, roster log); invitation tokens stored as
  SHA-256, single use, 48 h.
- One-time codes from another person are never copied or filled
  automatically, only from my own devices or a service.
- Group key rotation that excludes a device; old keys kept 7 days, then
  forgotten. Secrets zeroized in memory, never logged.
- Relay: Ed25519 challenge authentication with a domain prefix and
  single-use nonce; pairing tokens stored as SHA-256 only; bounded frames
  and envelopes; no content in logs; limits on unauthenticated connections
  and failures per IP (IPv6 per /64), in memory only; `X-Forwarded-For` read
  only from a declared front proxy; metrics on a separate local address;
  hardened systemd unit and a shell-less container image.
- Files: chunks encrypted under a per-file key (RustCrypto STREAM), SHA-256
  checked before a file becomes visible, signed HTTP requests (time window,
  single-use nonce), size and total quota, deletion after delivery or 48 h,
  received names sanitized.
- Android: secrets encrypted with AES-256-GCM by a non-exportable Keystore
  key, backups disabled, no cleartext traffic, only `http(s)` links opened.
- Extension: keys generated and used in WebAssembly, stored encrypted;
  messages accepted only from the extension's own pages; strict CSP; host
  permissions requested origin by origin; received files decrypted on the
  fly and shown sandboxed, never written to disk unless downloaded.
- One-time codes: otp-relay opens the mailbox read-only (`BODY.PEEK`, never
  marks mail read), keeps the app password in a `0600` file, masks codes in
  its logs and serves them on a `0600` Unix socket checked by peer UID; it
  trusts DMARC only from Gmail's own `Authentication-Results` header. The
  bridge forwards only the code, sender name and domain,
  DMARC result and expiry, never logs a code, and sends each code once. The
  extension fills a page by itself only for an authenticated sender, on a
  page of the same registrable domain (Public Suffix List, private suffixes
  included) over HTTPS; never a password field.
- Supply chain: `cargo deny` in CI (advisories, licenses, sources), CI
  actions pinned by commit.

## Known limits

- Metadata stays visible to the relay and its host.
- A single group key per group: no per-message forward secrecy.
- A saved file (Downloads) is plaintext, like any download.
- The extension briefly handles plaintext in JavaScript to pass it to
  WebCrypto.
- The host running otp-relay holds a mailbox app password, which grants
  full access to the mailbox, not just to codes.
- A received one-time code is copied to the clipboard, so it may land in an
  OS clipboard history. A bridge sending to my devices (`--to`) holds my
  group key; as a service (`--to-user`) it holds none.
- The admin is trusted for names and links: a person's name is the one the
  admin signed, and the relay host sees who is linked to whom.
- An invitation sent by SMS or chat can be read by the operator until it is
  used.
- Losing every admin device loses the anchor: `ot-server admin reset` on
  the relay, then every person must be invited again. Keep at least two
  admin devices.
- Automatic code filling needs access to all HTTPS pages (optional, granted
  on a click): a flaw in the extension would then reach every such page.
