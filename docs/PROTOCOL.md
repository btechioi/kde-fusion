# KDE Fusion control protocol (v1)

The control channel is intentionally small. It is used for peer discovery,
pairing, viewer presence, starting/stopping Moonlight and status queries. Video
and audio never pass through it: Sunshine and Moonlight carry the stream.

## Transport

- TCP, default port `47811`, bound to `0.0.0.0` (LAN only).
- One JSON object per line, UTF-8, newline terminated. Maximum line length is
  1 MiB; larger requests are rejected.
- Each message is wrapped in an envelope:

```json
{
  "version": 1,
  "request_id": "optional-correlation-id",
  "token": "optional 64 hex character bearer token",
  "message": { "type": "hello", "device_id": "...", "name": "...", "version": "...", "platform": "linux" }
}
```

`version` defaults to `1` when omitted. `request_id` is echoed in the response.
`token` is only sent on authenticated messages.

## Discovery

Peers advertise `_kdefusion._tcp.local.` over mDNS with these TXT properties:

| Key | Meaning |
| --- | --- |
| `device_id` | Stable random 32-hex-character id |
| `name` | Human readable device name |
| `version` | KDE Fusion version |
| `proto` | Protocol version (`1`) |

The SRV port is the control port. mDNS is a convenience only; a peer can also be
added by address through the API/CLI if discovery is blocked.

## Messages

| Type | Direction | Auth | Purpose |
| --- | --- | --- | --- |
| `hello` | either | no | Exchange device id, name, version, platform |
| `hello_ack` | server | no | Server identity |
| `pair_request` | initiator | no | Request pairing with a six-digit code |
| `pair_pending` | server | no | Pairing request id |
| `pair_poll` | initiator | no | Poll a pairing request |
| `pair_status` | server | no | `pending`, `approved`, `rejected`, `expired`, `not_found`; carries the token on approval |
| `pair_sunshine` | host | yes | Ask the viewer to run `moonlight pair` with a PIN |
| `pair_sunshine_result` | viewer | yes | Pairing success/failure |
| `info` | host | yes | Query the viewer's physical resolution/scale |
| `info_result` | viewer | yes | Resolution, scale, platform |
| `start_session` | host | yes | Ask the viewer to launch Moonlight |
| `stop_session` | host | yes | Ask the viewer to stop Moonlight |
| `status` | either | yes | Session status |
| `status_result` | either | yes | Session status payload |
| `ping` / `pong` | either | no | Liveness |
| `error` | either | no | `code` + `message` |

## Pairing

1. The initiator sends `pair_request` with its `device_id`, `name`, `platform`
   and a six-digit `code`.
2. The receiver creates a pending request with a 120-second lifetime and shows
   a KDE notification. The user approves or rejects it in the GUI/CLI.
3. The initiator polls with `pair_poll`. When approved, the receiver returns
   `pair_status` with `status: "approved"` and a fresh 256-bit token.
4. The receiver stores the token keyed by the initiator's device id; the
   initiator stores the returned token keyed by the receiver's device id.
5. Both sides persist a peer record (name, addresses, port, platform) in
   `state.json`. Tokens never go into `state.json`; they live in the Secret
   Service under `service=kdefusion key=peer-token-<device-id>`.

Removing a peer deletes its token. Re-pairing is required.

## Authentication

`validate_token` compares the presented token against every stored peer token
using a constant-time comparison. Tokens are 32 random bytes rendered as 64 hex
characters. A request with an unknown or revoked token receives
`error: auth`.

## Starting a session

The host resolves the peer from `state.json` and mDNS, opens a control
connection, and sends:

```json
{
  "version": 1,
  "request_id": "…",
  "token": "…",
  "message": {
    "type": "start_session",
    "session": {
      "session_id": "…",
      "mode": "extend",
      "layout": "right",
      "width": 1920,
      "height": 1080,
      "fps": 60,
      "bitrate_kbps": 40000,
      "audio": false,
      "codec": "HEVC",
      "app": "Desktop",
      "host_name": "Living Room PC"
    }
  }
}
```

The viewer validates arguments (no shell is involved), launches Moonlight
fullscreen and returns its session status. The host polls `status` every five
seconds. Three consecutive failures, an `idle` status or a `failed` status stop
the host session and restore the display layout.

## Threat model and limitations

- Anyone on the LAN can send `hello` and `pair_request`; they cannot start a
  session without an approved token.
- Pairing is user-approved by default. `pairing.auto_approve` exists for
  headless test rigs and should not be enabled on a shared network.
- The token is sent in clear text inside the LAN. v1 does not implement TLS.
  Do not expose port 47811 to the internet.
- The protocol cannot execute commands, read files or change arbitrary settings.
- The local HTTP API is loopback-only and unauthenticated.

## Versioning

The envelope carries a protocol version. A peer with a different version is
rejected with `error: version`. Future versions may add messages; receivers must
ignore unknown message types rather than crashing.
