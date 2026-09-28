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
| GET | `/system/slots` | yes | how many application slots the firmware knows about — see below |
| GET | `/system/booted` | yes | the slot the device booted from, or `/` when it could not tell — see below |
| GET | `/system/status/<slot>` | yes | one slot, as last seen — see below |
| GET | `/system/inspect/<slot>` | yes | one slot, rescanned — **expensive, see below** |
| GET | `/system/activate/countdown` | no | the remaining seconds while a countdown runs; otherwise redirected — see below |
| POST | `/system/activate/<slot>` | yes | `Activating` (200), or `400` with the reason |
| POST | `/system/upload/<slot>` | yes | `Upload successfully` (200), or `400` with the reason |

`<slot>` is one path segment naming an application slot: `factory` for the factory
image, or `0`, `1`, … for the OTA slots, counting from `0` for the first one. It
is required, and it has to be less than `slots - 1` (see `/system/slots` below);
anything else, `ota0` included, is answered with `400` and the name of the error.

**Paths are matched exactly, so a trailing slash is never optional.** The router
matches a route only when the whole path has been consumed: `/system/status/0`
works, `/system/status/0/` is a `404`, and so is `/system/activate` with no slot
at all.

### `POST /system/upload/<slot>`

Writes an application image into the slot named in the path. The request body
**is** the image — no multipart, no form field. `Content-Length` must be set and
the body must be an application image (the `.bin`, not the ELF), in
`FlashStorage::WRITE_SIZE` multiples. The body read timeout is one hour, which is
far more than the upload needs but keeps a stalled client from pinning the device.

The target partition is erased before the first byte is written, so an upload
takes as long as erasing and writing the image does. Two targets are refused
outright, with `OTA(Invalid)`: `factory`, which is written over USB only, and the
slot the device booted from, where the erase would take the code it is executing
with it. Which slot that is cannot always be worked out — see `/system/booted`
below — so a client should also refuse those two itself rather than wait for the
device to catch them.

Failures arrive in one of two ways, and the difference matters:

**A refusal, before a single byte of the body is read**, is a `400` carrying the
name of the `SystemError` that refused it:

| Token | Meaning |
|---|---|
| `Uploading` | another upload is already running |
| `OTA(Invalid)` | the target is `factory`, or the slot the device booted from |
| `OTA(NotSupported)` | there is no such slot, or `Content-Length` is 0 or not a multiple of `WRITE_SIZE` |

**Anything that goes wrong after that is answered with `200`.** The request was
accepted; what became of the image is reported by the `message` field of
`/system/progress` (and by the slot's own state, once it is read back):

| Token | Meaning |
|---|---|
| `OutOfMemory` | the image is larger than the target slot, or the stream ran long |
| `Incomplete` | the stream ended before `Content-Length` bytes arrived |
| `WriteFailure` | the connection failed while reading the body |
| `OTA(Invalid)` | the partition table has no such slot |
| `OTA(NotSupported)` | the image does not fit, or the flash refused the target |
| `OTA(OutOfBounds)` / `OTA(WriteProtected)` / `OTA(StorageError)` | the erase or the write itself failed |

**So a `200` does not mean the image is usable.** The response only says the
bytes were taken; whether the result can boot is a separate question, answered by
`/system/status/<slot>` — and if it cannot, `/system/progress` is the only place
the reason is written down.

### `POST /system/activate/<slot>`

Selects the slot named in the path as the next one to boot, starts a countdown
(5 s) and reboots. It is deliberately a `POST`: it changes what the device will
boot, and it cannot be undone from the network. `factory` is accepted and means
"clear the selection", which makes the next boot the factory image — the same
state a USB flash leaves behind.

`400` with a token is returned when the switch is refused: `NotSupported` or
`Invalid` when the partition table has no such slot, `OTA(InvalidImage)` when the
slot holds something that cannot boot, `Uploading` while an upload is running,
`OTADataInvalid` when the partition table has no OTA data partition, and `OTA(…)`
when that partition cannot be read or written.

**The check is made against the last known state of the slot, not against a fresh
scan**, and a slot nobody has asked about has no state to check — activating it
is refused with `OTA(InvalidImage)` even when it holds a perfectly good image.
So `GET /system/inspect/<slot>` has to come first (an upload into the slot does
the same thing). Both pages work that way: read the slot, then activate.

The firmware erases the OTA-data sector it is about to write before every switch,
and for `factory` it erases the whole partition instead: clearing the selection
means **both** entries have to be blank, and an entry that has been programmed
cannot be blanked by writing to it. An unreadable OTA-data partition is erased at
boot, so switching is safe to repeat. The crate underneath does not erase that
sector itself — see the README's known limitations for why the firmware has to do
it.

### `GET /system/slots` and `GET /system/booted`

`/system/slots` answers with a plain decimal number: how many application slots
the firmware has, factory included. The valid `<slot>` values are therefore
`factory` and the numbers `0` to `slots - 2`.

`/system/booted` answers with the index of the slot the device booted from, or
with `/` when it could not tell. It works that out by asking the MMU which flash
region is mapped and matching that against the partition table, which is not
always answerable, so **a client must not assume the device can refuse an upload
into the slot it is running from**: it refuses that slot when it knows which one
it is, and `factory` in every case, but on `/` the client is the only thing left
to catch it. A page that uploads should prefer an empty slot, and ask for
confirmation when the target is not empty and the device cannot say what it
booted from.

### `GET /system/status/<slot>` and `GET /system/inspect/<slot>`

Both answer with the same 12 fields (see below). They differ in whether the device
looks at the flash:

- `/system/status/<slot>` returns what the device already knows — the result of
  the last scan of that slot. It touches no flash; poll this one.
- `/system/inspect/<slot>` **scans the slot**: reads the image back out of flash,
  walks its segments, hashes it with SHA-256 and compares that against the digest
  appended to the image. That is the whole image read and hashed in software, on
  the one core the door control and the Wi-Fi stack also run on, so it takes a
  noticeable moment and competes with them. **Ask for it when the answer matters,
  not on a timer** — a page's "read" button, or once after an upload.

The scan is what fills the cache, and apart from an upload it is the only thing
that does: a slot nobody has asked about reads back as `Unchecked`.

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

**`/system/status/<slot>` and `/system/inspect/<slot>`**

```
<state>|<segment_count>|<entry_address>|<chip_id>|<hash_appended>|<secure_version>| \
<version>|<project>|<time>|<date>|<sha256>|<digest_sha256>
```

A `<slot>` the partition table does not have is answered with `400` and the name
of the error, never with a body. A body that is not exactly 12 fields means the
device's output buffer overflowed and the response was cut short: treat it as an
error, not as partial data to act on.

- `state` — `Unchecked`, `Empty`, `Unknown`, `Broken`, `Unverified`, `Foreign` or
  `Available`. Only `Unverified`, `Foreign` and `Available` can be activated, and
  `Unchecked` is what a slot nobody has scanned reads back as.
  - `Unchecked` — the device has not looked at this slot yet.
  - `Empty` — no image header where one would start.
  - `Unknown` — an ESP image, but not an application image: the header is there
    and the application descriptor is not.
  - `Broken` — an application image that fails its own structure or its digest.
  - `Unverified` — sound, but it carries no appended digest to check itself
    against.
  - `Foreign` — sound, but built by something other than this project.
  - `Available` — sound, and this project's.
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
(`POST /system/upload/<slot>`, `POST /system/activate/<slot>`, `GET /open`,
`GET /setopen?state=…`) should check the response's `redirected` flag, or the
final URL, before reporting success: a command issued during the countdown
otherwise appears to have succeeded although it never ran.

## Notes for clients

- One request per connection, no keep-alive.
- Authenticated endpoints cost a PBKDF2; a page that polls should stay at 1–2 s
  and then slow down.
- Poll `/system/status/<slot>`, never `/system/inspect/<slot>`: the first reads a
  cached value, the second reads and hashes the whole image on every call.
- While a countdown runs, everything is redirected to the reboot page. A client
  that polls the countdown should treat that redirect as the "switch finished"
  signal and **not follow it** into the reboot page: following costs a fresh
  authenticated page on every poll (a PBKDF2 each time), which is sufficient to
  slow the device down.
- The panel works with a plain `curl -u user:pass`; nothing needs a session or a
  cookie.
