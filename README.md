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

---

## Features

- **Web control panel** — open the door, or switch it to stay unlocked, from any
  browser on the same network.
- **Physical button** — opens the door for a few seconds without touching the
  network at all.
- **Fails safe** — the lock is controlled locally, so the door keeps working
  even when the Wi-Fi is down or the server is unreachable.
- **Password protection** — the control panel is guarded by HTTP Basic Auth.
  Passwords are stored as PBKDF2 hashes, so the plaintext never reaches the
  device.
- **mDNS** — reachable as `http://ayachinene.local` on the local network,
  without having to look up an IP address.
- **Configuration as a file** — network credentials and accounts live in a TOML
  file that is compiled into the firmware, so there is one place to edit and no
  runtime setup.

---

## Hardware

| Part | Notes |
|---|---|
| ESP32-S3 board | Developed against the ESP32-S3-WROOM-1 |
| Relay module | Drives the lock |
| Electric lock | 12 V fail-secure (locked when unpowered) recommended |
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
- `GPIO5` uses the chip's internal pull-up, so the button wires straight to GND
  and a press produces a falling edge. No external resistor is needed.

Avoid the pins reserved for the UART console (`GPIO43`/`GPIO44`), USB
(`GPIO19`/`GPIO20`), and the SPI flash. If you change the pins, update them in
`src/main.rs`.

---

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

---

## Configuration

The firmware is configured by a TOML file that is read by `build.rs` at compile
time. Start from the template:

```bash
cp server_config_template.toml server_config.toml
```

Then open `server_config.toml` and fill in, at minimum:

- `wifi.ssid` and `wifi.password` — the network to join.
- `users.signed` — at least one account for the control panel. See the comments
  in the template for the exact syntax; an empty list means nobody can log in.
- `users.salt` — replace the placeholder with your own string.

Every field has an explanatory comment in the template.

`server_config.toml` is listed in `.gitignore`, so a checkout always builds
against your own copy.

### How secrets are handled

`build.rs` reads the plaintext passwords and emits **PBKDF2-HMAC-SHA256 hashes**
into the generated code. Only the hashes are compiled into the firmware, so a
dumped flash image does not reveal the passwords.

This protects the passwords *at rest*. It does **not** protect them in transit:
the control panel is served over plain HTTP, so anyone on the same network can
sniff the credentials. Treat the panel as trustworthy only on a network you
control.

---

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

If the serial port is not detected automatically, pass it explicitly:

```bash
espflash flash --monitor --port COM5
```

### Using a prebuilt binary

If you would rather not set up the Espressif toolchain, the [Releases] page
carries a prebuilt `.bin` image. It is an ordinary ESP32-S3 binary image, with
nothing about it specific to this project's build setup, so **any** flashing
tool will take it as it is — esptool, the Arduino IDE or PlatformIO, a
board-vendor GUI flasher, whatever you already use for the chip. Nothing to
build locally, and nothing to convert first.

> The configuration is baked in at build time, so a prebuilt image is tied to
> the network and the accounts it was built with.

[Releases]: https://github.com/SCU-Maker-Org/Ayachi-door-access/releases

---

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

### Troubleshooting

| Symptom | Likely cause |
|---|---|
| No serial port detected | Charge-only USB cable, or a missing driver |
| `NoAccessPointFound` in the log | Wrong SSID, or the access point is 5 GHz only — the ESP32-S3 is 2.4 GHz only |
| Connected, but no IP | The access point is not handing out DHCP leases |
| `ayachinene.local` does not resolve | Check the environment of the tool you are using to open the address. |
| Nothing on the panel, page loads | The device is fine; check the serial log |

---

## Project layout

```
src/
├── main.rs                 startup: init, spawn tasks, keep Wi-Fi connected
└── ayachi_core/
    ├── mod.rs              shared config constants, spawn helper
    ├── door.rs             door state machine and lock hardware
    ├── network.rs          Wi-Fi controller and the embassy-net stack
    ├── mdns.rs             mDNS responder
    └── server.rs           HTTP routes and Basic Auth
build.rs                    reads server_config.toml, generates constants
index.html                  the control panel, compiled into the firmware
server_config_template.toml the configuration template
```

The lock is owned by a single task (`door_task`). Other parts of the firmware
never touch the relay pin; they send it requests through signals instead. That
keeps the lock's state transitions serialised, so there is no way for two
callers to fight over it.

---

## Known limitations

- No OTA firmware updates yet. Flashing requires a USB connection.
- No watchdog. A panic halts the device until it is power-cycled, and if it
  panics while the lock is released, the door stays unlocked.
- HTTP Basic Auth over plain HTTP is only as private as the network it runs on.

---

## Credits

Written by **Han_feng**, 2026.

Based on the earlier ESP8266 door access project for the SCU Makerspace.
