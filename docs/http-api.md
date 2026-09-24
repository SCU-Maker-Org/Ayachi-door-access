# HTTP API

The device runs a small HTTP server on the LAN. The address and port come from
the configuration (`device.name` is also the mDNS hostname and the auth realm);
the serial log prints the IP it got. Every response carries
`Connection: close`, so a client makes one request per TCP connection.

## Authentication

Everything except the paths marked **no** in the tables below sits behind HTTP
Basic Auth (`WWW-Authenticate: Basic realm="<device name> Login"`). Accounts are
the ones in `users.signed` of the config, and the password hash is PBKDF2 with
`users.rounds` iterations — so **every authenticated request costs the device a
full PBKDF2 computation**. Poll slowly, and prefer one request over two.

A failed login is `401` with the `WWW-Authenticate` header.

## Door

| Method | Path | Auth | Response |
|---|---|---|---|
| GET | `/ciallo` | no | `Ciallo～(∠·ω< )⌒★` |
| GET | `/` | yes | the control panel (HTML) |
| GET | `/logo` | no | the logo (`image/webp`) |
| GET | `/favicon.ico` | no | the favicon (`image/x-icon`) |
| GET | `/status` | no | `open` or `lock` — the door's actual state |
| GET | `/setopen?state=0` | yes | `Door set to locked` |
| GET | `/setopen?state=1` | yes | `Door set to opened` |
| GET | `/setopen?state=<other>` | yes | `400 Invalid state` |
| GET | `/open` | yes | `Successfully open door for once` |

The three commands only *ask* `door_task` to act, through signals, and reply
immediately: the door may still be moving when the response arrives. Read
`/status` if you need to know where it ended up.

## Firmware

| Method | Path | Auth | Response |
|---|---|---|---|
| GET | `/system/` | yes | status page (HTML) |
| GET | `/system/maintenance` | yes | update page (HTML) |
| GET | `/system/resetting` | no | shown while the device reboots (HTML) |
| GET | `/system/progress` | yes | upload progress, one line — see below |
| GET | `/system/status/?slot=<slot>` | yes | one partition — see below |
| GET | `/system/status/all` | yes | three partitions, `factory`, `current`, `next` in that order |
| GET | `/system/activate/countdown` | no | `Some(n)` while the reboot is counting down, `None` otherwise |
| POST | `/system/activate/` | yes | `Activating` (200), or `400` with the reason |
| POST | `/system/upload` | yes | `Upload successfully` (200), or `400` with the reason |

`<slot>` is `factory`, `current` or `next`, lowercase; it is required, and an
unknown value never reaches the handler.

The trailing slash on `/system/status/` and `/system/activate/` is optional:
`/system/status?slot=next` is the same request as `/system/status/?slot=next`.

### `POST /system/upload`

The request body **is** the image — no multipart, no form field. `Content-Length`
must be set and the body must be an application image (the `.bin`, not the ELF),
in `FlashStorage::WRITE_SIZE` multiples. The body read timeout is one hour, which
is far more than the upload needs but keeps a stalled client from pinning the
device.

Failures come back as `400` with a short token, e.g.:

| Token | Meaning |
|---|---|
| `Uploading` | another upload is already running |
| `OutOfMemory` | the image is larger than the target slot, or the stream ran long |
| `Incomplete` | the stream ended before `Content-Length` bytes arrived |
| `WriteFailure` | the connection failed while reading the body |
| `OTA(Invalid)` | the partition table has no second application slot |

The response only says the bytes were written. **Whether the result is usable is
a separate question** — ask `/system/status/?slot=next` afterwards.

### `POST /system/activate/`

Switches the boot slot and starts a countdown (5 s), after which the device
reboots. It is deliberately a `POST`: it changes what the device will boot, and
it cannot be undone from the network.

`400` with a token is returned when there is nothing to activate
(`OTA(NotSupported)` for no second slot, `OTA(InvalidImage)` when the slot holds
something that cannot boot, `Uploading` while an upload is running).

### Response formats

Three endpoints answer with Rust's `{:?}` of an internal struct, so the shape is
that struct's `Debug` implementation — see `impl Debug for UploadProcess` and
`impl Debug for PartitionInfo` in `src/ayachi_core/system.rs`. Fields are joined
with `|`, in the order below.

**`/system/progress`**

```
<uploading>|<state>|<message>|[erase_cur, erase_total]|[write_cur, write_total]
```

- `uploading` — `true` or `false`. Without it, a state like `Writing` cannot be
  told apart from "stopped during the write".
- `state` — `Idle`, `Received`, `Erasing`, `Writing`, `Identifying`, `Ready` or
  `Failed`. Only the last two are final; anything else with `uploading=false`
  means the upload died there.
- `message` — `None`, or the error the upload ended with.
- byte counts in decimal.

**`/system/status/?slot=<slot>` and `/system/status/all`**

```
<state>|<segment_count>|<entry_address>|<chip_id>|<hash_appended>|<secure_version>| \
<version>|<project>|<time>|<date>|<sha256>|<digest_sha256>
```

The single-slot form is wrapped in `Some(...)` or `None`; the `/all` form is an
array of three of those.

- `state` — `Unavailable`, `Empty`, `Unknown`, `Broken`, `Unverified`,
  `Foreign` or `Available`. Only `Unverified`, `Foreign` and `Available` are
  worth activating.
- `version`, `project`, `time` and `date` are NUL-padded ASCII as a decimal byte
  array (`Some([48, 46, 49, 46, 48, 0, …])` for `"0.1.0"`).
- `sha256` is the ELF digest recorded in the application descriptor;
  `digest_sha256` is the digest of the image as it sits in flash. A client that
  wants to check an upload can strip the last 32 bytes of the `.bin` when
  `hash_appended` is true and hash that.
- `hash_appended` says whether the image carries that appended digest. When it is
  false the device has nothing to verify against, which is why those slots come
  back as `Unverified`.

## Redirects

A small layer sits in front of everything else:

- While the activation countdown is running, **any** path other than the two
  unauthenticated system paths above is answered with `303` to
  `/system/resetting`. This is what lets a browser that is still on the panel
  follow the reboot instead of showing an error.
- When no countdown is running, a request for one of those two paths is answered
  with `303` to `/system/maintenance` — the reboot page is only reachable during
  a reboot, and the countdown endpoint only during the countdown.

## Notes for clients

- One request per connection, no keep-alive.
- Authenticated endpoints cost a PBKDF2; a page that polls should stay at 1–2 s
  and then slow down.
- The panel works with a plain `curl -u user:pass`; nothing needs a session or a
  cookie.
