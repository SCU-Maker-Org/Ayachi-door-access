# Ayachi Door Access

<p align="center">
  <img src="nene.ico" alt="Ayachi" width="160">
</p>

A network-controlled door lock for the SCU Makerspace, written in pure Rust for the
ESP32-S3.

It is a ground-up rewrite of an earlier ESP8266 / Arduino project, with a few
long-standing bugs fixed along the way. The device joins the local Wi-Fi
network, serves a small web control panel, and can also be operated from a
physical button next to the door.

## Features

- **Web control panel** — open the door, or switch it to stay unlocked, from any
  browser on the same network.
- **Physical button** — opens the door for a few seconds without touching the
  network at all.
- **Firmware updates over Wi-Fi** — a maintenance page takes a new application
  image, verifies what actually landed in the flash, and switches the device
  over to it on the next boot. The running firmware is never written over, and
  an image it cannot verify is refused unless you say so explicitly.
- **Works without the network** — the lock is controlled locally, so the door
  keeps working even when the Wi-Fi is down or the server is unreachable.
- **Password protection** — the control panel is guarded by HTTP Basic Auth.
  Passwords are stored as PBKDF2 hashes, so the plaintext never reaches the
  device.
- **mDNS** — reachable as `http://ayachinene.local` on the local network,
  without having to look up an IP address.
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
- The bolt hangs on the relay's **normally-closed** contact, so it is powered —
  and therefore holding — while the relay is idle. A power cut releases it: this
  is not a fail-secure lock, and a power-free door is an open door.
- `GPIO5` uses the chip's internal pull-up, so the button wires straight to GND
  and a press produces a falling edge. No external resistor is needed.

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

We suggest copying it to `config/server_config.toml`: that is the path
`.cargo/config.toml` supplies by default, so the build picks it up on its own,
with no extra flag to add. It is listed in `.gitignore`, so a checkout always
builds against your own copy. To build against a file somewhere
else instead, see [Environment variables](#environment-variables) below.

Writing one from the template, the least you have to do is fill in the Wi-Fi
credentials — the one section it marks `[]` — and add at least one account:

- `wifi.ssid` and `wifi.password` — the network to join.
- `users.signed` — at least one account for the control panel. See the comments
  in the template for the exact syntax; an empty list means nobody can log in.

Two more are a decision rather than a box to fill in:

- `users.salt` — the template ships a placeholder; we suggest picking your own, so
  that the value is not shared with every other build of this project. Changing
  it rewrites the generated password hashes.
- `users.rounds` — the PBKDF2 cost. Higher is harder to brute-force and slower to
  log in, and with HTTP Basic Auth every request pays it. The value in the
  template is a safe starting point; how far to push it is your call.

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

This builds, flashes over USB, and opens a serial monitor. **Use `--release`** —
debug builds can be orders of magnitude slower and may misbehave on timing
sensitive parts.

To build without flashing:

```bash
cargo build --release
```

If the serial port is not detected automatically, pass it explicitly — and keep
the partition-table flags:

```bash
espflash flash --monitor --port COM5 \
  --partition-table config/memory_partitions.csv --target-app-partition origin
```

> **Always pass the partition table.** It is not baked into the image, so a bare
> `espflash flash` writes its own default single-factory table over the one in
> flash. That table has no second application slot, which silently takes
> Wi-Fi updates away. `cargo run` already supplies both flags (they live in
> `.cargo/config.toml`); only hand-written `espflash` commands have to add them.

### Using a prebuilt binary

If you would rather not set up the Espressif toolchain, the [Releases] page
carries a prebuilt `.bin` image. It is an ordinary ESP32-S3 binary image, with
nothing about it specific to this project's build setup, so **any** flashing
tool will take it as it is — esptool, the Arduino IDE or PlatformIO, a
board-vendor GUI flasher, whatever you already use for the chip. Nothing to
build locally, and nothing to convert first.

> The configuration is baked in at build time, so a prebuilt image is tied to
> the network and the accounts it was built with.

A released `.bin` is also what the maintenance page takes, so a device that
already runs this firmware can be moved to a newer release over Wi-Fi, without
a cable.

Each release also carries the matching ELF next to the image. It is not for
flashing — flash the `.bin` — but it is what decodes the log stream and turns a
panic backtrace into function names, and it only works paired with the image from
the same release.

[Releases]: https://github.com/SCU-Maker-Org/Ayachi-door-access/releases

## Environment variables

Both of the build's knobs live in `.cargo/config.toml`, and both are defaults: an
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
only dumps bytes, shows that as noise. `cargo run` hands the ELF to espflash,
which decodes it; standalone, do the same (for a prebuilt image, the ELF attached
to its release):

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
2. **Read what landed.** The device reads the image back out of the flash and
   reports the version, project name, build time and digests it finds. If the
   image carries an appended digest the device has already checked it; if it
   does not, the page shows the digest it computed so you can compare it with
   the one the release publishes.
3. **Activate.** The device switches the boot slot, counts down a few seconds,
   and reboots.

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
| Upload refused, "not supported" | The partition table in flash has no second app slot — reflash with `--partition-table` |
| Upload finishes but the image is `broken` | The file was not an application `.bin`, or it was truncated on the way |

## Project layout

```
src/
├── main.rs                 startup: init, spawn tasks, keep Wi-Fi connected
└── ayachi_core/
    ├── mod.rs              shared config constants, spawn helper
    ├── door.rs             door state machine and lock hardware
    ├── network.rs          Wi-Fi controller and the embassy-net stack
    ├── mdns.rs             mDNS responder
    ├── server.rs           HTTP routes, Basic Auth, request layers
    ├── system.rs           flash partitions, OTA upload / activation
    └── utils.rs            small helpers (stack strings, format helpers)
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
keeps the lock's state transitions serialised, so there is no way for two
callers to fight over it.

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
- No watchdog. A panic halts the device until it is power-cycled, and if it
  panics while the lock is released, the door stays unlocked.
- HTTP Basic Auth over plain HTTP is only as private as the network it runs on.

## Credits

Written by **Han_feng**, 2026.

Based on the earlier ESP8266 door access project for the SCU Makerspace.
