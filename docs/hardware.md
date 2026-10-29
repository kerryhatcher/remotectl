# Hardware reference

## Kit specs

| Item | Value |
|------|-------|
| MCU | ESP32-S3R8, dual-core Xtensa LX7 up to 240 MHz |
| Memory | 8 MB PSRAM, 8 MB flash |
| Wireless | 2.4 GHz Wi-Fi, BLE 5.0 / Mesh; LoRa via Semtech SX1262, 862–930 MHz |
| Power | USB-C 5 V in; battery in 4.2 V; charge current 100 mA |
| Operating temp | -40 °C to 65 °C (kit page) |
| Size | 21 x 17.8 mm (XIAO); Wio board stacks on it |
| Antennas | Wi-Fi/BT (U.FL) and LoRa (U.FL) included. Seeed says the stock FPC antenna is "only for testing" |

The XIAO ESP32S3 and the Wio-SX1262 board join through a 30-pin board-to-board
connector (DF40, "XIAO-Add-On" J3 on the XIAO schematic). The Wio board also
brings out 2x 1x7 headers (J1/J2).

## Buttons and LEDs

| Control | Board | Pin | Notes |
|---------|-------|-----|-------|
| **R** (reset) | XIAO | `CHIP_PU` | Tiny SMD button beside the USB-C port, left side |
| **B** (boot) | XIAO | `GPIO0` | Tiny SMD button beside the USB-C port, right side. Hold during reset to enter the ROM bootloader |
| User button | Wio-SX1262 | `GPIO21` | The single, larger button on the Wio board. Active low, 10 kΩ pull-up. **This is not BOOT.** Meshtastic uses it as the "program button" |
| User LED | XIAO | `GPIO21` | Yellow, shares the pin with the Wio user button |
| Charge LED | XIAO | `CHG-LED` | Red, not software controllable |
| User LED | Wio-SX1262 | `GPIO48` | Green, 330 Ω to GND, active high |

See `images/xiao-esp32s3-front.png` for the R/B button positions and
`images/wio-sx1262-extension-board.png` for the Wio board.

## LoRa radio (SX1262) pins

Verified against Meshtastic's `seeed_xiao_s3` variant (`variant.h`). The Wio
schematic only gives net names (`LORA_*`) at the board-to-board connector,
so the GPIO numbers come from Meshtastic.

| SX1262 signal | ESP32-S3 GPIO | XIAO label |
|---------------|---------------|------------|
| SCK | 7 | D8 |
| MISO | 8 | D9 |
| MOSI | 9 | D10 |
| NSS / CS | 41 | D12 |
| NRST | 42 | D11 |
| DIO1 (IRQ) | 39 | — |
| BUSY | 40 | — |
| RF_SW (RXEN) | 38 | — |
| DIO2 | internal | Drives the antenna switch inside the module (see [wio-sx1262-module.md](wio-sx1262-module.md)) |
| DIO3 | internal | Powers the TCXO, 1.8 V |

Meshtastic also lists `GPIO1` for the L76K GPS standby and `GPIO43`/`GPIO44` (TX/RX)
for an optional GPS. These are not part of this kit.

## XIAO ESP32S3 header pins

| XIAO | GPIO | Alternates |
|------|------|-----------|
| D0 | 1 | ADC, TOUCH1 |
| D1 | 2 | ADC, TOUCH2 |
| D2 | 3 | ADC, TOUCH3 |
| D3 | 4 | ADC, TOUCH4 |
| D4 | 5 | SDA |
| D5 | 6 | SCL |
| D6 | 43 | TX |
| D7 | 44 | RX |
| D8 | 7 | SCK |
| D9 | 8 | MISO |
| D10 | 9 | MOSI |
| D11 | 42 | |
| D12 | 41 | |

Pads D11 and D12 are the same GPIOs the Wio board uses for NRST and NSS.

## Other Wio-SX1262 connector pins (from the Wio schematic)

Besides the radio signals, the Wio board routes these XIAO GPIOs to its
connector: `GPIO14`–`GPIO18`, `GPIO47`, `GPIO48` (LED) and `GPIO21` (button).
`VIN` is the 5 V rail and `+3V3` powers the module.

## Adafruit 1.14" TFT display (ADA6113)

240x135 IPS, ST7789 driver, SPI. Adafruit guide:
<https://learn.adafruit.com/adafruit-1-14-240x135-color-newxie-tft-display>.
The board has no reset pin, so the driver does a software reset. It takes
3.3–5 V power and has its own regulator and level shifter.

### Wiring (as used by `firmware/`)

The display shares the radio's SPI bus (SCK, MOSI) and has its own CS and DC.

| Display pin | Function | XIAO pin | GPIO |
|-------------|----------|----------|------|
| V+ | Power | 3V3 | — |
| G | Ground | GND | — |
| CL | SPI clock | D8 / SCK | 7 |
| DA | SPI data (MOSI) | D10 / MOSI | 9 |
| CS | Chip select | D6 | 43 |
| DC | Data/command | D7 | 44 |
| BL | Backlight | not connected (on by default; tie to GND to turn off) | — |

D5 (GPIO6) is left free as the only ADC-capable pin the Wio board passes through.
GPIO43 and GPIO44 are the chip's UART pins. Logs use the USB serial/JTAG port, so they are free.

### Header layout

With the USB-C port pointing west:

```
                      NORTH row
        5V/VIN  GND   3V3   D10    D9    D8    D7
         │      │      │     │      │     │     │
         │     [G]   [V+]   [DA]   ·    [CL]  [DC]
   ┌─────┴──────┴──────┴─────┴──────┴─────┴─────┴──┐
USB-C ◄ │                                           │
(west)  └─────┬──────┬──────┬─────┬──────┬─────┬───┘
              │      │      │     │      │     │     │
              ·      ·      ·     ·      ·    [PT]  [CS]
             D0     D1     D2    D3     D4    D5    D6
                      SOUTH row
```

On the Wio board the same positions are labeled `VIN, GND, 3V3, MOSI, MISO, SCK, D7`
(north) and `DIO1, BUSY, RST, NSS, RF-SW, D5, D6` (south). `[PT]` marks where a
phototransistor would go.

### Firmware settings

| Setting | Value |
|---------|-------|
| SPI | Shared SPI2, mode 0, 10 MHz |
| Display crate | `mipidsi` 0.10, `ST7789` model |
| Size / offset | 135x240 native portrait, offset (52, 40) |
| Rotation | `Deg0` (portrait) |
| Color | `ColorInversion::Inverted` |

The first run needs the border to line up with the panel edge. If it doesn't, adjust
`DISPLAY_OFFSET_X` and `DISPLAY_OFFSET_Y` in `firmware/src/bin/main.rs`.
