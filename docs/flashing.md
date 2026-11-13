# Flashing

The kit ships with Meshtastic. Flashing custom firmware replaces it.
Restore Meshtastic from <https://flasher.meshtastic.org> (board "Seeed Xiao ESP32-S3").
Meshtastic marks this variant as needing DFU (`requires_dfu = true`), so it
expects the manual bootloader method below.

The board enumerates as an ESP32-S3 USB Serial/JTAG device (`303a:1001`) on
`/dev/ttyACM0`. The `dialout` group gives access without sudo.

## Entering the bootloader

The XIAO ESP32S3 has two tiny SMD buttons next to the USB-C connector,
labeled **R** (reset) and **B** (boot) on the silkscreen. They are easy to
miss. See `images/xiao-esp32s3-front.png`. The single larger button on the
Wio-SX1262 board is the user button on `GPIO21`. It does **not** enter the
bootloader.

From the XIAO wiki (<https://wiki.seeedstudio.com/xiao_esp32s3_getting_started/>):

- **Method 1:** Hold **B**, plug in the USB cable, then release **B**.
- **Method 2:** Hold **B**, press and release **R**, then release **B**.
- If an upload fails or the port is missing, press **R** first. If that doesn't help, use the bootloader steps above.

Use a pointed tool such as a pen tip or paperclip for the buttons.

## espflash

With the link keys exported, `cargo run -p xiao-firmware` in `firmware/` runs `espflash flash --monitor --chip esp32s3` (set in
`firmware/.cargo/config.toml`). Build artifacts are stored under the workspace root `target/` directory. Source `~/export-esp.sh` first.

```
espflash board-info             # check the connection
espflash monitor                # serial console only
espflash erase-flash            # wipe flash (removes Meshtastic and its settings)
```

`espflash` asks to confirm the serial port and whether to remember it. The port is `/dev/ttyACM0`.

## Legacy Wi-Fi firmware recovery

The older HID firmware used the USB pins (GPIO19/20) as a USB device, replacing the
USB-Serial-JTAG port. Those builds did not expose the current CDC recovery interface.
The legacy network-based script below applies only to the old Wi-Fi firmware:

- `firmware/flash.sh` builds, calls `POST /bootloader` (needs `DEVICE_IP` and `API_TOKEN`),
  waits for `/dev/ttyACM0` and retries `espflash flash --no-stub` up to 5 times. The ROM
  download mode over this USB path is flaky, so retries are normal.
- Manual: hold **B** while plugging in, then `espflash flash --chip esp32s3 --port
  /dev/ttyACM0 <elf>`. This is the reliable route. Release **B** and tap **R** afterwards
  to start the app (the chip stays in the bootloader while GPIO0 is low).
- Do not clear `RTC_CNTL_USB_CONF` bits to force the JTAG PHY before reset. That hung the
  chip and needed the manual method to recover.

## Log

- 2026-10-29: first `cargo run` failed with `Failed to connect to the device`
  while Meshtastic was running. The board re-enumerated but no flash happened.
- 2026-10-30: USB HID keyboard firmware. `POST /bootloader` reaches download mode, but
  flashing through it failed on some attempts (stub: `MemData` timeout, no-stub: broken pipe).

## USB-command recovery (experimental)

The XIAO firmware source provides a composite USB HID and CDC interface. A host can send
`BOOTLOADER` followed by a newline to its CDC port to request ROM download mode. To build
and flash using this path, set `RECOVERY_PORT` to the XIAO application's CDC port and
`ROM_PORT` to its ROM serial port, then run `firmware/flash-usb.sh`. The ports may differ.
The current XIAO firmware must already include the CDC recovery interface for this to work.

Source the link key file and the ESP toolchain environment before using either script.
For the Heltec V3, set `PORT` to its CP2102 serial port and run
`heltec-firmware/flash-usb.sh`; `espflash` requests bootloader mode through the USB serial
bridge's reset lines. The Heltec auto-reset path has been verified by reflashing the connected board. The XIAO
USB reset and one button-free flash succeeded, but later attempts returned a `FlashEnd` error
and required a USB cable reconnection. Button-free reflashing is not yet reliable.
