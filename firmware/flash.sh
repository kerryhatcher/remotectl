#!/usr/bin/env bash
# Build and flash over USB while the firmware runs as a USB keyboard.
#
# The keyboard firmware owns the USB port, so `espflash` cannot reset the board. This
# asks the running firmware to reboot into the ROM download mode (POST /bootloader), then
# flashes. The ROM's USB-OTG downloader is flaky, so the flash step is retried.
#
#   DEVICE_IP=192.168.1.159 ./flash.sh
#
# Needs WIFI_SSID, WIFI_PASSWORD and API_TOKEN in the environment (same as `cargo build`).
# If the board is not reachable over Wi-Fi, hold B on the XIAO while plugging it in and
# run `espflash flash --chip esp32s3 target/xtensa-esp32s3-none-elf/debug/firmware`.
set -euo pipefail
cd "$(dirname "$0")"

: "${DEVICE_IP:?set DEVICE_IP to the board's address}"
: "${API_TOKEN:?set API_TOKEN}"
PORT="${PORT:-/dev/ttyACM0}"
BIN=target/xtensa-esp32s3-none-elf/debug/firmware

cargo build

for attempt in 1 2 3 4 5; do
    echo "== attempt $attempt: rebooting into download mode"
    curl -s -m 5 -o /dev/null -X POST -H "Authorization: Bearer $API_TOKEN" \
        "http://$DEVICE_IP/bootloader" || true

    for _ in $(seq 10); do
        [ -e "$PORT" ] && break
        sleep 1
    done
    sleep 1

    if espflash flash --chip esp32s3 --port "$PORT" --no-stub "$BIN"; then
        echo "== flashed"
        exit 0
    fi
    sleep 3
done

echo "== giving up; hold B while plugging the board in and flash by hand" >&2
exit 1
