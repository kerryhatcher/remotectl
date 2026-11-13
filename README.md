# remotectl

remotectl sends keyboard and mouse commands from a host CLI through a Heltec WiFi LoRa 32 V3
and a 915 MHz LoRa link to a Seeed XIAO ESP32-S3. The XIAO presents a USB keyboard and mouse
to the controlled computer. The active firmware does not use Wi-Fi.

## Workspace

| Directory | Package | Purpose |
| --- | --- | --- |
| `firmware/` | `xiao-firmware` | XIAO LoRa receiver, USB HID and USB recovery |
| `heltec-firmware/` | `heltec-firmware` | Heltec V3 USB serial and LoRa sender |
| `cli/` | `remotectl` | Host command-line tool |
| `shared/` | `remotectl-shared` | `no_std` command, radio and packet definitions |

The boards were tested with an authenticated LoRa ping and a short keyboard text command.
Mouse movement and scrolling have been sent to the hardware and registered as GNOME input
activity; pointer position and scrolling behavior have not been visually confirmed. A left-click
was verified by a change of focus between terminal panes. The XIAO's USB-command reset works, but
ROM flashing has intermittently failed at `FlashEnd` and required reconnecting its USB cable.
See [flashing instructions](docs/flashing.md) before updating either board.

## Setup

Attach the 915 MHz antennas to both radios. Connect the Heltec's USB-C port to the computer
running the CLI and the XIAO's USB-C port to the computer to be controlled. The XIAO also uses
the Wio SX1262 and the display wiring described in [hardware.md](docs/hardware.md).

Create a private key file outside the repository. The generator requires Python's `cryptography`
package and refuses to overwrite an existing file:

```sh
mkdir -p -m 700 ~/.config/remotectl
python3 scripts/generate-link-keys.py ~/.config/remotectl/link.env
source ~/.config/remotectl/link.env
source ~/export-esp.sh
```

Use the same link key for both builds, the signing key only for the Heltec build, and the
verification key for the XIAO build. Do not commit the key file. The keys are compiled into
the firmware and can be extracted by someone with access to an unprotected device or its build
artifacts; hardware-backed key protection is not configured.

Build from each firmware directory so Cargo picks up its ESP32-S3 target configuration:

```sh
(cd firmware && cargo build -p xiao-firmware)
(cd heltec-firmware && cargo build -p heltec-firmware)
```

The flash scripts build first and require the relevant exported keys:

```sh
PORT=/dev/ttyUSB1 ./heltec-firmware/flash-usb.sh
RECOVERY_PORT=/dev/ttyACM0 ROM_PORT=/dev/ttyACM0 ./firmware/flash-usb.sh
```

Use the actual ports for your setup. The XIAO recovery script requires firmware already exposing
its CDC recovery interface; the first installation may need an existing bootloader path.
Firmware artifacts are written under the workspace root's `target/xtensa-esp32s3-none-elf/`.

## Use

```sh
export REMOTECTL_PORT=/dev/ttyUSB1
cargo run -p remotectl -- ping
cargo run -p remotectl -- type 'Hello from remotectl'
cargo run -p remotectl -- move -- -3 42
cargo run -p remotectl -- click left
cargo run -p remotectl -- scroll -- -2
```

`move` uses signed relative HID counts; the host may apply pointer acceleration. `click` supports
`left`, `right`, and `middle`, with optional `--double`. `scroll` accepts signed steps from -127
to 127.

`type` accepts up to 120 ASCII bytes per request, using the US keyboard layout. Text goes to
whatever has focus on the controlled computer. A successful command means the receiver accepted
it into a USB input queue; it does not guarantee the host application processed the input. A
timeout leaves execution uncertain, so retrying may duplicate input.

Each radio command is encrypted with XChaCha20-Poly1305 and signed with Ed25519. The receiver
uses a fresh random challenge to reject replays and returns a challenge-bound authenticated
acknowledgment. Radio jamming and denial of service remain possible. This protocol is experimental
and has not had an independent security review. The old Wi-Fi HTTP implementation is retained as
inactive source; its [documentation](docs/api.md) describes legacy behavior only.
