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
`/status` for the resulting state.

## Firmware

| Method | Path | Auth | Response |
|---|---|---|---|
| GET | `/system/` | yes | status page (HTML) |
| GET | `/system/maintenance` | yes | update page (HTML) |
| GET | `/system/resetting` | no | shown while the device reboots (HTML) |
| GET | `/system/progress` | yes | upload progress, one line — see below |
| GET | `/system/status?slot=<slot>` | yes | one partition — see below |
| GET | `/system/activate/countdown` | no | the remaining seconds while a countdown runs; otherwise redirected — see below |
| POST | `/system/activate/` | yes | `Activating` (200), or `400` with the reason |
| POST | `/system/upload` | yes | `Upload successfully` (200), or `400` with the reason |

`<slot>` is `factory`, `current` or `next`, lowercase; it is required, and an
unknown value never reaches the handler.

**Paths are matched exactly, so a trailing slash is never optional.** The router
matches a route only when the whole path has been consumed: `/system/status?slot=next`
works, `/system/status/?slot=next` is a `404`. The one endpoint that *needs* the
trailing slash is `POST /system/activate/` — it is registered as the root of a
nested router, so `/system/activate/` is its entire path and `/system/activate`
is a `404`.

### `POST /system/upload`

The request body **is** the image — no multipart, no form field. `Content-Length`
must be set and the body must be an application image (the `.bin`, not the ELF),
in `FlashStorage::WRITE_SIZE` multiples. The body read timeout is one hour, which
is far more than the upload needs but keeps a stalled client from pinning the
device.

Failures come back as `400` with a short token — the name of the `SystemError`
the upload ended with:

| Token | Meaning |
|---|---|
| `Uploading` | another upload is already running |
| `OutOfMemory` | the image is larger than the target slot, or the stream ran long |
| `Incomplete` | the stream ended before `Content-Length` bytes arrived |
| `WriteFailure` | the connection failed while reading the body |
| `OTA(NotSupported)` | there is nowhere to write: the partition table has no second application slot, **or** the OTA data partition could not be read (see the note below), or the flash refused the target |
| `OTA(OutOfBounds)` / `OTA(WriteProtected)` / `OTA(StorageError)` | the erase or the write itself failed |

`OTA(NotSupported)` deserves its own note: the target slot is chosen from the OTA
data partition, so **anything that makes that partition unreadable looks exactly
like "no second slot"** — including the case where it holds bytes that are
neither erased nor a valid entry (a stale region left behind by an older
partition table, for instance). If an upload is refused with `NotSupported` while
the partition table clearly has two application slots, inspect the OTA data
partition rather than reflashing.

The response only says the bytes were written. **Whether the result is usable is
a separate question** — ask `/system/status?slot=next` afterwards.

### `POST /system/activate/`

Switches the boot slot and starts a countdown (5 s), after which the device
reboots. It is deliberately a `POST`: it changes what the device will boot, and
it cannot be undone from the network.

`400` with a token is returned when there is nothing to activate
(`OTA(NotSupported)` for no second slot or an unreadable OTA data partition,
`OTA(InvalidImage)` when the slot holds something that cannot boot, `Uploading`
while an upload is running).

One caveat, written out in the README's known limitations: activating a third
time since the last USB flash is known to corrupt the OTA data partition (a
defect in the crate this firmware builds its updates on). The device remains
operational, but further uploads are refused until that partition is erased.

### Response formats

These endpoints answer with plain text assembled by the `impl CompactFormat`
blocks in `src/ayachi_core/system.rs` (`UploadProcess`, `PartitionInfo`) plus the
`Option`/array/text helpers in `src/ayachi_core/utils.rs`. Two conventions hold
everywhere:

- **a field with no value is the single character `/`** (that is an
  `Option::None`, not an empty string);
- **arrays are joined with `;` and carry no brackets** — `[u8; 32]` digests come
  out as `240;185;24;…`, `[u32; 2]` byte counts as `0;0`.

Fields are joined with `|`. No field can contain a `|`: the text helper replaces
`|`, `/` and `;` inside `version`, `project`, `time` and `date` with `.`, and
error names never contain one. Splitting on `|` is therefore always safe.

**`/system/progress`**

```
<uploading>|<state>|<message>|erase_cur;erase_total|write_cur;write_total
```

- `uploading` — `true` or `false`. Without it, a state like `Writing` cannot be
  told apart from "stopped during the write".
- `state` — `Idle`, `Received`, `Erasing`, `Writing`, `Identifying`, `Ready` or
  `Failed`. Only the last two are final; anything else with `uploading=false` means the
  upload terminated in that state.
- `message` — `/` when the upload has no error, otherwise the error name, e.g.
  `OTA(NotSupported)`.
- byte counts are decimal, in bytes.

**`/system/status?slot=<slot>`**

```
<state>|<segment_count>|<entry_address>|<chip_id>|<hash_appended>|<secure_version>| \
<version>|<project>|<time>|<date>|<sha256>|<digest_sha256>
```

If the slot is not in the partition table at all, the whole body is `/`. A body
that is not exactly 12 fields means the device's output buffer overflowed and the
response was cut short: treat it as an error, not as partial data to act on.

- `state` — `Unavailable`, `Empty`, `Unknown`, `Broken`, `Unverified`,
  `Foreign` or `Available`. Only `Unverified`, `Foreign` and `Available` can be
  activated.
- `version`, `project`, `time` and `date` are plain ASCII text, printed up to the
  first NUL (they are NUL-padded in flash).
- `hash_appended` is `1` or `0`; it says whether the image carries an appended
  digest. When it is `0` the device has nothing to verify the image against,
  which is why those slots come back as `Unverified`.
- `sha256` is the ELF digest recorded in the application descriptor;
  `digest_sha256` is the digest of the image as it sits in flash. Both are
  decimal bytes joined with `;`. A client that wants to check an upload can strip
  the last 32 bytes of the `.bin` when `hash_appended` is `1` and hash the rest.

## Redirects

A small layer sits in front of everything else:

- While the activation countdown is running, every path other than these four is
  answered with `303` to `/system/resetting`: `/system/resetting`,
  `/system/activate/countdown`, `/logo` and `/favicon.ico`. The reboot page
  needs its own two paths, and its two assets, to remain reachable while the
  device is still answering; this is what lets a browser that is still on the
  panel follow the reboot instead of showing an error.
- When no countdown is running, a request for `/system/resetting` or
  `/system/activate/countdown` is answered with `303` to `/system/`: the reboot
  page is reachable only during a reboot, and the countdown endpoint only during
  the countdown.

A `303` is followed automatically by browsers and by `fetch`, and the redirected
request never reaches its handler, yet the client observes the target's `200`.
**A client that inspects only the status code cannot distinguish "the handler
ran" from "the request was redirected"**, so every request that changes state
(`POST /system/upload`, `POST /system/activate/`, `GET /open`,
`GET /setopen?state=…`) should check the response's `redirected` flag, or the
final URL, before reporting success: a command issued during the countdown
otherwise appears to have succeeded although it never ran.

## Notes for clients

- One request per connection, no keep-alive.
- Authenticated endpoints cost a PBKDF2; a page that polls should stay at 1–2 s
  and then slow down.
- While a countdown runs, everything is redirected to the reboot page. A client
  that polls the countdown should treat that redirect as the "switch finished"
  signal and **not follow it** into the reboot page: following costs a fresh
  authenticated page on every poll (a PBKDF2 each time), which is sufficient to
  slow the device down.
- The panel works with a plain `curl -u user:pass`; nothing needs a session or a
  cookie.
