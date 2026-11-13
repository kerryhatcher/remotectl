#!/usr/bin/env bash
# The stock Heltec V3 CP2102 serial bridge controls reset/boot through modem lines.
set -euo pipefail
cd "$(dirname "$0")"
: "${PORT:?set PORT to the Heltec CP2102 serial port}"
: "${REMOTECTL_LINK_KEY:?set REMOTECTL_LINK_KEY before building}"
: "${REMOTECTL_SIGN_KEY:?set REMOTECTL_SIGN_KEY before building}"

cargo build -p heltec-firmware
exec espflash flash --chip esp32s3 --port "$PORT" \
    ../target/xtensa-esp32s3-none-elf/debug/heltec-firmware
