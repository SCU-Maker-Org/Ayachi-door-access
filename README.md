# Ayachi Door Access

<p align="center">
  <img src="nene.ico" alt="Ayachi" width="160">
</p>

A network-controlled door lock for the SCU Makerspace, written in pure Rust for the
ESP32-S3.

It is a ground-up rewrite of an earlier ESP8266 / Arduino project, with several
long-standing bugs fixed in the process. The device joins the local Wi-Fi
network, serves a small web control panel, and can also be operated from a
physical button next to the door.

## Features

- **Web control panel** — open the door, or switch it to stay unlocked, from any
  browser on the same network.
- **Physical button** — opens the door for a few seconds without involving the
  network.
- **Firmware updates over Wi-Fi** — a maintenance page takes a new application
  image, verifies the image actually written to flash, and switches the device
  over to it on the next boot. The running image is never overwritten, and an
  image it cannot verify is refused unless explicitly confirmed.
- **Works without the network** — the lock is controlled locally, so the door
  keeps working even when the Wi-Fi is down or the server is unreachable.
- **Password protection** — the control panel is guarded by HTTP Basic Auth.
  Passwords are stored as PBKDF2 hashes, so the plaintext never reaches the
  device.
- **mDNS** — reachable as `http://ayachinene.local` on the local network, with
  no IP address lookup required.
- **Configuration as a file** — network credentials and accounts live in a TOML
  file that is compiled into the firmware, so there is one place to edit and no
  runtime setup.

## Hardware

| Part | Notes |
|---|---|
| ESP32-S3 board | Developed against the ESP32-S3-WROOM-1 |
| Relay module | Drives the lock |
| Electric lock | 12 V electric bolt, powered through the relay's normally-closed contact |
| Momentary push button | Mounted outside the door |
| Power supply | 5 V / 2 A or better — do not power it from a PC USB port |

### Wiring

```
ESP32-S3                     External
────────                     ────────
GPIO4  ────────────────────→ Relay IN      (relay drives the lock)
GPIO5  ────────────────────→ Button ──────→ GND   (pressing pulls GPIO5 low)
```

- `GPIO4` high → relay energised → lock released.
- The bolt is wired to the relay's **normally-closed** contact, so it is powered,
  and therefore locked, while the relay is idle; a power cut releases it. The
  lock is fail-open: an unpowered door is an unlocked door.
- `GPIO5` uses the chip's internal pull-up, so the button connects directly to
  GND and a press produces a falling edge; no external resistor is required.

Avoid the pins reserved for the UART console (`GPIO43`/`GPIO44`), USB
(`GPIO19`/`GPIO20`), and the SPI flash. If you change the pins, update them in
`src/main.rs`.

## Requirements

### Hardware

An ESP32-S3 board and a USB **data** cable. Note that many USB cables are
charge-only and will never be recognised by the host — if `espflash` reports
"no serial ports could be detected", try another cable first.

### Software

- **Rust with the Espressif toolchain.** The target is `xtensa-esp32s3-none-elf`,
  which is not supported by upstream Rust, so Espressif maintains a fork. It is
  installed with [`espup`](https://github.com/esp-rs/espup):

  ```bash
  cargo install espup
  espup install
  ```

  This installs a toolchain named `esp`, which `rust-toolchain.toml` selects
  automatically when you build inside this directory.

  > On Windows you may also need to run `export-esp.sh` / `export-esp.ps1` from
  > your profile to put the toolchain on `PATH`.

- **`espflash`**, the flashing tool:

  ```bash
  cargo install espflash
  ```

- A working Cargo mirror if you are behind a slow connection — `Cargo.lock` is
  gitignored, so the first build resolves the dependency versions itself.

## Configuration

The firmware is configured by a TOML file that `build.rs` reads at compile time.
It has to be shaped like `config/server_config_template.toml` — the same
sections and field names — and that template is where every field is explained.

Copy it to `config/server_config.toml`: that is the path
`.cargo/config.toml` supplies by default, so the build picks it up on its own,
with no extra flag to add. It is listed in `.gitignore`, so a checkout always
builds against your own copy. To build against a file somewhere
else instead, see [Environment variables](#environment-variables) below.

At minimum, a configuration must supply the Wi-Fi credentials — the section the
template marks `[]` — and at least one account:

- `wifi.ssid` and `wifi.password` — the network to join.
- `users.signed` — at least one account for the control panel. See the comments
  in the template for the exact syntax; an empty list means nobody can log in.

Two further fields require a decision rather than a value:

- `users.salt` — the template ships a placeholder; choosing your own is
  recommended, so that the value is not shared with other builds of this
  project. Changing it rewrites the generated password hashes.
- `users.rounds` — the PBKDF2 cost. A higher value is harder to brute-force and
  slower to authenticate, and with HTTP Basic Auth every request pays that cost.
  The template value is a safe starting point; raising it further is a trade-off
  to measure.

### How secrets are handled

`build.rs` reads the plaintext passwords and emits **PBKDF2-HMAC-SHA256 hashes**
into the generated code. Only the hashes are compiled into the firmware, so a
dumped flash image does not reveal the passwords.

This protects the passwords *at rest*. It does **not** protect them in transit:
the control panel is served over plain HTTP, so anyone on the same network can
sniff the credentials. Treat the panel as trustworthy only on a network you
control.

## Building and flashing

Connect the board and run:

```bash
cargo run --release
```

This builds, flashes over USB, and opens a serial monitor. **Use `--release`**:
debug builds can be an order of magnitude slower and can misbehave in
timing-sensitive code.

To build without flashing:

```bash
cargo build --release
```

If the serial port is not detected automatically, name it explicitly and keep
the partition-table flags:

```bash
espflash flash --monitor --port COM5 \
  --partition-table config/memory_partitions.csv --target-app-partition origin \
  --erase-parts otadata
```

> **Always pass the partition table.** It is not baked into the image, so a bare
> `espflash flash` writes its own default single-factory table over the one in
> flash. That table has no second application slot, which disables Wi-Fi updates
> without reporting an error. `cargo run` already supplies these flags (they live
> in `.cargo/config.toml`); only hand-written `espflash` commands have to add
> them.

> **Clear the OTA boot selection as well** (`--erase-parts otadata`). `espflash`
> never touches the OTA data partition, so without this a USB flash has no effect
> once an update has been activated: the bootloader keeps booting the slot
> recorded there. Erasing it leaves the partition empty — the bootloader's
> "factory boot condition" — so that a USB flash determines the boot slot again.
> Both `cargo run` and `sundries/flash_scu.bat` already do this.

### Using a prebuilt binary

If setting up the Espressif toolchain is not desirable, the [Releases] page
carries a prebuilt `.bin` image. It is an ordinary ESP32-S3 binary image, with
nothing about it specific to this project's build setup, so **any** flashing
tool accepts it as it is: esptool, the Arduino IDE or PlatformIO, or a
board-vendor GUI flasher. No local build and no conversion is required.

> The configuration is baked in at build time, so a prebuilt image is tied to
> the network and the accounts it was built with.

A released `.bin` is also what the maintenance page takes, so a device that
already runs this firmware can be moved to a newer release over Wi-Fi, without
a cable.

Each release also carries the matching ELF next to the image. It is not for
flashing — flash the `.bin` — but it decodes the log stream and resolves panic
backtraces into function names, and it is valid only when paired with the image
from the same release.

[Releases]: https://github.com/SCU-Maker-Org/Ayachi-door-access/releases

## Environment variables

Both build settings live in `.cargo/config.toml`, and both are defaults: an
environment variable of the same name overrides what is set there.

### DEFMT_LOG

The log filter, `info` by default. It is applied at **compile time** — messages
below the level are not in the image at all — so changing it means rebuilding, not
reconfiguring:

```bash
DEFMT_LOG=debug cargo run --release
```

The stream itself is defmt-encoded: the device sends short references, and the
mapping back to message text comes from the ELF. A plain serial monitor, which
only dumps bytes, shows that as noise. `cargo run` passes the ELF to espflash,
which decodes it; when running standalone, do the same (for a prebuilt image,
the ELF attached to its release):

```bash
espflash monitor --elf target/xtensa-esp32s3-none-elf/release/ayachi-door-access
```

### SERVER_CONFIG

Which config file `build.rs` reads; the default is `config/server_config.toml`.
To build against a file somewhere else:

```bash
SERVER_CONFIG=your_path_to_server_config cargo run --release
```

Make sure that file contains TOML laid out like the template — `build.rs` parses
it without looking at the name. Relative paths resolve against the crate root, and
switching files re-runs `build.rs`, so the firmware always carries the config you
named.

## Usage

Once the device has joined the network it prints its address to the serial
monitor:

```
[INFO ] Connected to wifi, IP: 192.168.1.50
```

Open that address in a browser:

```
http://192.168.1.50/
```

or, where mDNS is supported:

```
http://ayachinene.local/
```

The browser will prompt for a username and password — use one of the accounts
from `users.signed`.

The pages are the only interface most people need. If you want to drive the
device from a script, or debug it with `curl`, the endpoints are written down in
[`docs/http-api.md`](docs/http-api.md).

### Control panel

| Button | Effect |
|---|---|
| **Open once** | Releases the lock for a few seconds, then locks again automatically |
| **Stay unlocked** | Holds the lock released until told otherwise |
| **Lock** | Re-locks immediately and leaves always-unlocked mode |

The status line at the top refreshes automatically.

### Physical button

Pressing the button behaves like "Open once", and works regardless of the state
of the network.

### Firmware updates over Wi-Fi

The device can replace its own application image. Start at `/system/maintenance`
(the footer of the control panel links to it) and:

1. **Upload** the application image — the `.bin` from a release, not the ELF.
   The progress bar follows the erase and then the write; the page polls the
   device, so it is fine to refresh or reopen it mid-upload.
2. **Read what landed.** Click **Read** on the backup-partition card — nothing is
   fetched from the device until then. It reports the version, project name,
   build time and the digests it finds in the image. If the image carries an
   appended digest the device has already checked it; if it does not, the card
   shows the digest the device read back from flash, for you to compare with the
   one the release publishes.
3. **Activate.** The device switches the boot slot, counts down a few seconds,
   and reboots. The page follows the reboot and returns to the control panel on
   its own. If the new image never comes back, reflash over USB — that also
   clears the boot selection, so the device boots the freshly flashed `origin`.

Uploads always go into the *inactive* slot — the firmware that is running is
never written over, and nothing is switched until you activate. If a new image
fails the bootloader's own validation, the bootloader falls back to the factory
slot (`origin`), so a refused image does not leave the device unusable.

The page asks for an explicit confirmation before activating an image that is
*not* one this project built, or one it could not verify — moving to either is
not something the device can undo by itself. The page shown while it reboots,
and the countdown it polls, are deliberately left unauthenticated so a reboot
does not prompt for the password again.

> **This feature is new.** It has been written but not yet exercised on real
> hardware, so treat Wi-Fi updates as experimental for now.

### Troubleshooting

| Symptom | Likely cause |
|---|---|
| No serial port detected | Charge-only USB cable, or a missing driver |
| `NoAccessPointFound` in the log | Wrong SSID, or the access point is 5 GHz only — the ESP32-S3 is 2.4 GHz only |
| Connected, but no IP | The access point is not handing out DHCP leases |
| `ayachinene.local` does not resolve | Check the environment of the tool you are using to open the address. |
| Nothing on the panel, page loads | The device is fine; check the serial log |
| Upload refused, "not supported" | Either the partition table in flash has no second app slot (reflash with `--partition-table`) or the OTA data partition is unreadable — see the otadata item under [Known limitations](#known-limitations) |
| Upload finishes but the image is `broken` | The file was not an application `.bin`, or it was truncated on the way |
| Upload slower than expected | Each progress poll pays a PBKDF2, on the same core that is erasing and writing the flash — polling competes with the update. Poll less often, or exempt `/system/progress` from auth |

## Project layout

```
src/
├── main.rs                 startup: init, spawn tasks, keep Wi-Fi connected
└── ayachi_core/
    ├── mod.rs              shared config constants
    ├── door.rs             door state machine and lock hardware
    ├── network.rs          Wi-Fi controller and the embassy-net stack
    ├── mdns.rs             mDNS responder
    ├── server.rs           HTTP routes, Basic Auth, request layers
    ├── system.rs           flash partitions, OTA upload / activation
    └── utils.rs            small helpers (stack strings, format helpers, task spawning)
assets/
├── index.html              the control panel
├── maintenance.html        the firmware update page
├── system.html             partition and progress details
├── resetting.html          shown while the device reboots
└── pic/                    logo and favicon
config/
├── server_config_template.toml   the configuration template
└── memory_partitions.csv         the partition table (factory + 2 OTA slots)
docs/
└── http-api.md             the HTTP endpoints, for scripts and clients
build.rs                    reads the config, generates constants
```

The lock is owned by a single task (`door_task`). Other parts of the firmware
never touch the relay pin; they send it requests through signals instead. That
keeps the lock's state transitions serialised, so two callers cannot contend for
the lock.

Updates follow the same shape: the HTTP handler only writes the image into the
inactive slot, and a separate task (`system_task`) owns the switch-over and the
reboot. That is what lets the browser finish its request before the device
disappears.

## Known limitations

- Wi-Fi updates need the app-slot partition table. A board flashed with
  `espflash`'s default table has a single application slot and cannot update
  itself — see [Building and flashing](#building-and-flashing).
- There is no rollback yet: the firmware does not confirm an activated image as
  working, and the bootloader `espflash` ships has rollback disabled. An image
  that boots but misbehaves — say, one that never gets on the network — has to be
  replaced over USB.
- **Activating a few times in a row can corrupt the OTA data partition.** This is
  a defect in the `esp-bootloader-esp-idf` crate this firmware builds its update
  path on: it rewrites that partition without erasing the target sector first,
  and NOR flash can only clear bits, so after a couple of switches the sequence
  number can no longer be represented. In practice the **third activation since
  the last USB flash** writes an invalid entry: the slot pages then report a read
  failure and later uploads are refused with `OTA(NotSupported)`, while the
  device itself keeps working from `origin`. `espflash erase-region 0xf000
  0x2000` clears it. The fix is to erase the target 4 KB half before switching —
  or to own the OTA data partition outright — and it belongs with multi-slot
  support, below.
- **Multi-slot is planned.** The partition table has room for two more
  application slots in the upper half of the 16 MB flash, and updating a *named*
  slot (instead of "the inactive one") is the reason the OTA data partition has
  to be handled by us rather than by the crate.
- Single core. The scheduler is started on the first core and the second core is
  left parked, so the Wi-Fi stack, the HTTP server and the update work all share
  one core. Dual-core support is expected later.
- No watchdog. A panic halts the device until it is power-cycled, and if it
  panics while the lock is released, the door stays unlocked.
- HTTP Basic Auth over plain HTTP is only as private as the network it runs on.

## Credits

Written by **Han_feng**, 2026.

Based on the earlier ESP8266 door access project for the SCU Makerspace.
