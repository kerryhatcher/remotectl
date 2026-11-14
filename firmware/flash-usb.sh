#!/usr/bin/env bash
# Requires firmware that exposes the USB CDC recovery interface.
set -euo pipefail
cd "$(dirname "$0")"
: "${RECOVERY_PORT:?set RECOVERY_PORT to the XIAO USB CDC serial port}"
: "${ROM_PORT:?set ROM_PORT to the XIAO ROM bootloader serial port}"

: "${REMOTECTL_LINK_KEY:?set REMOTECTL_LINK_KEY before building}"
: "${REMOTECTL_VERIFY_KEY:?set REMOTECTL_VERIFY_KEY before building}"

cargo build -p xiao-firmware
stty -F "$RECOVERY_PORT" 115200 raw -echo
printf 'BOOTLOADER\n' > "$RECOVERY_PORT"

# The ROM device may have a different path from the application's CDC device.
for _ in $(seq 1 30); do
    if lsusb -d 303a:0009 | grep -q . && [ -c "$ROM_PORT" ]; then
        if espflash flash --chip esp32s3 --port "$ROM_PORT" --no-stub \
            ../target/xtensa-esp32s3-none-elf/debug/firmware; then
            exit 0
        fi
        printf '%s\n' 'Flash failed; retrying once without resetting the ROM bootloader.' >&2
        exec timeout 25s espflash flash --chip esp32s3 --port "$ROM_PORT" --no-stub \
            --before no-reset ../target/xtensa-esp32s3-none-elf/debug/firmware
    fi
    sleep 1
done
printf '%s\n' 'ROM bootloader did not appear; no flash was attempted.' >&2
exit 1
