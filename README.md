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
It has to be shaped like `server_config_template.toml` — the same sections and
field names — and that template is where every field is explained.

Copy it to `server_config.toml`, next to it in the crate root: that is the path
`.cargo/config.toml` supplies by default, so the build picks it up on its own,
with no extra flag to add. It is listed in `.gitignore`, so a checkout always
builds against your own copy. To build against a file somewhere
else instead, see [Environment variables](#environment-variables) below.

At minimum, a configuration must supply the Wi-Fi credentials — the section the
template marks `[]` — and at least one account:

- `wifi.ssid` and `wifi.password` — the network to join.
- `users.signed` — at least one account for the control panel. See the comments
  in the template for the exact syntax; an empty list means nobody can log in.

Three further fields require a decision rather than a value:

- `users.salt` — the template ships a placeholder; choosing your own is
  recommended, so that the value is not shared with other builds of this
  project. Changing it rewrites the generated password hashes.
- `users.rounds` — the PBKDF2 cost. A higher value is harder to brute-force and
  slower to authenticate, and with HTTP Basic Auth every request pays that cost.
  The template value is a safe starting point; raising it further is a trade-off
  to measure.
- `device.panic_retry` — how many panics in a row the device tolerates before it
  gives up on the running image and returns to the factory one; see
  [Panic handling](#panic-handling). The template value rides out a one-off
  fault and still gives up on an image that panics on every boot.

The `[memory]` section is a decision of a different kind: it says how much of the
board this project takes, and how that space is divided. The cargo runner turns it
into the partition table it flashes, so building through `cargo run` keeps the two
in step; a hand-written `espflash` command has to pass a table that agrees — see
[Building and flashing](#building-and-flashing).

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

**Use `--release`**: debug builds can be an order of magnitude slower and can
misbehave in timing-sensitive code — and the runner `cargo run` needs is only
built in a release profile, so without the flag there is nothing to run.

### What `cargo run` does

Beyond building, three things happen in the cargo runner
(`tools/ayachi_runner.rs`, compiled by `build.rs` into `target/ayachi_runner`):

1. **It generates the partition table** from the `[memory]` section of your
   config (there is no table in the repository to keep in step by hand) and saves
   it next to the artifacts it builds, in `target/ayachi-door-access/<version>/`
   — `<version>` being the one in `Cargo.toml`:

   ```
   target/ayachi-door-access/<version>/
   ├── ayachi-door-access-v<version>.bin   the application image
   ├── ayachi-door-access-v<version>.elf   the same, with symbols for the log
   └── memory_partitions.csv               the table it flashed
   ```

2. **It flashes with that table**, into the factory slot
   (`--target-app-partition origin`).
3. **It clears the OTA boot selection** (`--erase-parts otadata`), so a USB flash
   is authoritative over any update activated earlier — without it the bootloader
   keeps booting the slot recorded in otadata, and the flash appears to do
   nothing.

> **A partition table has to be passed.** It is not baked into the application
> image, so a bare `espflash flash` writes its own default single-factory table
> over the one in flash — a table with no second application slot, which disables
> Wi-Fi updates without reporting an error. `cargo run` passes the generated one;
> anything else has to name one.

### Building without flashing

```bash
cargo build --release
```

A plain build produces no partition table and no image copy: generating those is
the runner's job, not the compiler's. To flash what you built by hand — for
example on a port `cargo run` does not pick up — pass the table from an earlier
`cargo run` of the same version:

```bash
espflash flash --monitor --port COM5 \
  --partition-table target/ayachi-door-access/<version>/memory_partitions.csv \
  --target-app-partition origin --erase-parts otadata
```

`cargo run` adds the last two flags itself; only a hand-written command has to
spell them out.

### Using a prebuilt binary

If setting up the Espressif toolchain is not desirable, the [Releases] page
carries a prebuilt `.bin` image. It is an ordinary ESP32-S3 binary image, with
nothing about it specific to this project's build setup, so **any** flashing
tool accepts it as it is: esptool, the Arduino IDE or PlatformIO, or a
board-vendor GUI flasher. No local build and no conversion is required.

That `.bin` is an *application* image, so it carries no partition table: it is
what goes into a board that already runs this firmware. A board that has never
been flashed needs the `-merged.bin` from the same release instead — the
whole-flash image, bootloader and partition table included, written from `0x0`.

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

Which config file `build.rs` reads; the default is `server_config.toml`, in the
crate root. To build against a file somewhere else:

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
2. **Check what landed.** Pick the slot under **Target slot**, then click
   **Read** on its card. The device keeps no per-slot information until it is
   asked, and that click is the asking: it reads the image back out of flash and
   hashes it, so it takes a moment — read when you mean to, not in a loop. The
   card reports the version, project name, build time and the digests it finds in
   the image. If the image carries an appended digest the device has already
   checked it; if it does not, the card shows the digest the device read back from
   flash, for you to compare with the one the release publishes.
3. **Activate.** The device switches the boot slot, counts down a few seconds,
   and reboots. The page follows the reboot and returns to the control panel on
   its own. If the new image never comes back, reflash over USB — that also
   clears the boot selection, so the device boots the freshly flashed `origin`.

Uploads go into the slot you name, and only into it. Each upload erases its
target before writing, which is why the device refuses two slots: the factory
image (`origin`), which is written over USB only, and the slot it booted from,
since erasing that erases the code it is running. It cannot always tell which
slot it booted from; when it cannot, the page asks you to confirm a target that
is not empty. Nothing is switched until you activate. If a new image fails the
bootloader's own validation, the bootloader falls back to the factory slot
(`origin`), so a refused image does not leave the device unusable.

An image that boots but keeps panicking does not need the cable either: the panic
counter (see [Panic handling](#panic-handling)) abandons it after
`device.panic_retry` panics, and the device returns to the factory image. An
image that boots cleanly and then simply does not do its job — one that never
gets on the network, say — never panics, so nothing counts it; the door still
opens from the button, but remote access and this page need a cable.

The same slots are listed one card at a time on `/system/`, and every card has an
activate button of its own — on the factory card it reads **Return to factory
settings**, which clears the boot selection, so the next boot is the factory
image: the state a USB flash leaves behind.

Both pages ask for an explicit confirmation before activating an image that is
*not* one this project built, or one it could not verify — moving to either is
not something the device can undo by itself. The page shown while it reboots,
and the countdown it polls, are deliberately left unauthenticated so a reboot
does not prompt for the password again.

### Troubleshooting

| Symptom | Likely cause |
|---|---|
| No serial port detected | Charge-only USB cable, or a missing driver |
| `NoAccessPointFound` in the log | Wrong SSID, or the access point is 5 GHz only — the ESP32-S3 is 2.4 GHz only |
| Connected, but no IP | The access point is not handing out DHCP leases |
| `ayachinene.local` does not resolve | Check the environment of the tool you are using to open the address. |
| Nothing on the panel, page loads | The device is fine; check the serial log |
| Upload refused, `OTA(Invalid)` | The target is the factory image, or the slot the device booted from — choose another slot |
| Upload refused, `OTA(NotSupported)` | The slot is past the last one, or the file length is not a multiple of the flash write size |
| Upload answered `200`, but the progress line ends in `OTA(Invalid)` | The partition table in flash has no such slot: `espflash` writes its default single-app table unless you pass `--partition-table`, or the firmware was built for more slots than the table has |
| Activation refused, `OTA(InvalidImage)` | That slot holds something that cannot boot — upload an application image into it |
| Upload finishes but the image is `broken` | The file was not an application `.bin`, or it was truncated on the way |
| Upload slower than expected | Each progress poll pays a PBKDF2, on the same core that is erasing and writing the flash — polling competes with the update. Poll less often, or exempt `/system/progress` from auth |
| A reboot with a backtrace at the end of the log | A panic: the firmware printed where it happened and rebooted. Nothing to do unless it repeats |
| The device reboots, reboots again, and comes back on the factory image | It panicked `device.panic_retry` times in a row, so the boot selection was cleared. Read the panic in the log, fix the image, and update again |

## Project layout

```
src/
├── main.rs                 startup: init, spawn tasks, keep Wi-Fi connected
└── ayachi_core/
    ├── mod.rs              generated constants, panic handling, factory reset
    ├── door.rs             door state machine and lock hardware
    ├── network.rs          Wi-Fi controller and the embassy-net stack
    ├── mdns.rs             mDNS responder
    ├── server.rs           HTTP routes, Basic Auth, request layers
    ├── system.rs           flash partitions, OTA upload / activation
    └── utils.rs            small helpers (stack strings, format helpers, task spawning)
assets/
├── index.html              the control panel
├── maintenance.html        the firmware update page
├── system.html             the slots, the progress, per-slot activation
├── resetting.html          shown while the device reboots
└── pic/                    logo and favicon
docs/
└── http-api.md             the HTTP endpoints, for scripts and clients
tools/
└── ayachi_runner.rs        the cargo runner: builds the partition table, flashes
build.rs                    reads the config, generates constants, builds the runner
server_config_template.toml the configuration template
                            (`server_config.toml` next to it is yours, gitignored)
```

The partition table is not a source file: the cargo runner generates it from the
`[memory]` section and hands it to `espflash` — see
[Building and flashing](#building-and-flashing).

The lock is owned by a single task (`door_task`). Other parts of the firmware
never touch the relay pin; they send it requests through signals instead. That
keeps the lock's state transitions serialised, so two callers cannot contend for
the lock.

Updates follow the same shape: the HTTP handler only writes the image into the
slot it was given, and a separate task (`system_task`) owns the switch-over and
the reboot. That is what lets the browser finish its request before the device
disappears.

## Panic handling

A panic — or a hardware fault, which the firmware turns into one — prints the
message and a backtrace, then reboots. It does not halt the device, and it does
not leave the lock in whatever state it was in: the lock pin is driven again on
the way up, so a device that panicked comes back locked.

Each panic also adds to a counter kept in an RTC register that survives a
software reset. That counter is what makes a boot loop recoverable: when it
reaches `device.panic_retry`, the next boot erases the OTA boot selection and the
bootloader falls back to the factory image. The bootloader `espflash` ships has
its own rollback support disabled, so the counting happens in the firmware. The
counter is cleared when an update is activated, so a freshly activated image
starts counting from zero.

## Known limitations

- Multi-core is planned. Everything runs on one core currently: the scheduler starts
  on the first core and leaves the second one parked, so the Wi-Fi stack, the
  HTTP server and the update work all share it.
- The panic counter is only cleared when an update is activated, not on a
  successful boot, so unrelated one-off panics months apart add up: enough of
  them return the device to the factory image with nothing wrong with the image
  it was running.
- HTTP Basic Auth over plain HTTP is only as private as the network it runs on.

## Credits

Written by **Han_feng**, 2026.

Based on the earlier ESP8266 door access project for the SCU Makerspace.
