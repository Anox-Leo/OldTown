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
| A group member | Read and write like the others, add a device, rotate the group key: a group serves one person. |
| A stolen device | Everything a member can do, until the group key is rotated without it and it is revoked on the relay. Android keys are encrypted by an Android Keystore key and excluded from backups, but usable while the phone is locked. Extension keys are encrypted by a non-extractable WebCrypto key of the same browser profile: whoever opens the computer session can use them. |

## In place

- End-to-end encryption (`crates/ot-crypto`, RustCrypto primitives only):
  XChaCha20-Poly1305 under a group key per epoch; key transport with X25519
  and HKDF-SHA256; Ed25519 signature of every message; associated data
  covering header, `msg_id`, sender, recipients and creation time. Format
  frozen by test vectors cross-checked against an independent
  implementation.
- Clients reject plaintext, senders outside the roster and bad signatures;
  trust comes from pairing, never from the relay's device list.
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
- Supply chain: `cargo deny` in CI (advisories, licenses, sources), CI
  actions pinned by commit.

## Known limits

- Metadata stays visible to the relay and its host.
- A single group key per group: no per-message forward secrecy.
- A saved file (Downloads) is plaintext, like any download.
- The extension briefly handles plaintext in JavaScript to pass it to
  WebCrypto.
