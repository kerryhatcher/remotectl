# Wio-SX1262 module notes

Source: `pdfs/Wio-SX1262_Module_Datasheet.pdf` (V1.1) and the Meshtastic variant.

## Firmware-relevant facts

- **RF switch:** TX/RX switching is decided by the SX1262's **DIO2** pin
  (DIO2 high = transmit, low = receive). Configure DIO2 as the RF switch
  (`SetDio2AsRfSwitchCtrl`).
- **TCXO:** The module has an active TCXO powered from **DIO3**. Enable it with
  `SetDio3AsTcxoCtrl` before calibrating or transmitting. Meshtastic uses 1.8 V.
  The datasheet says the TCXO voltage must stay at least 200 mV below VCC and spans 1.7–3.3 V.
- **RF_SW pin (module pin 1):** An extra external IO that controls the
  module's internal RF gate. Meshtastic drives it as `RXEN` on GPIO38.
- **BUSY:** Only send a new command when BUSY is low.
- **DC-DC mode:** The module uses the SX1262's DC-DC regulator, so call
  `SetRegulatorMode(DC-DC)`.
- **SPI:** 4-wire SPI, mode 0. The reset value of the LoRa sync word register
  at `0x0740` reads `0x1424`, which is a handy "is the chip alive" check.

## Electrical

| Item | Min | Typ | Max | Unit |
|------|-----|-----|-----|------|
| Supply (VCC) | 1.8 | 3.3 | 3.6 | V |
| Abs. max supply | -0.5 | | 3.9 | V |
| Frequency | 862 | | 930 | MHz |
| TX power | | | 22 | dBm |
| TX current @ 22 dBm | | 125 | | mA |
| RX current (BW 125 kHz) | | 7.6 | | mA |
| Sleep current | | 1.62 | | µA |
| Sensitivity, SF12 BW 125 kHz | | -136.73 | | dBm |
| RF input level | | | +10 | dBm |

LoRa bandwidth range is 7.8–500 kHz. The module also supports (G)FSK.
Antenna output is 50 Ω to a U.FL/IPEX connector. Do not transmit without an antenna attached.

## Module pinout (12-pin SMT)

| # | Name | Description |
|---|------|-------------|
| 1 | RF_SW | External IO controlling the internal RF gate |
| 2 | MISO | SPI MISO |
| 3 | MOSI | SPI MOSI |
| 4 | SCK | SPI clock |
| 5 | NRST | Reset, active low |
| 6 | NSS | SPI chip select |
| 7 | GND | |
| 8 | VCC | 3.3 V |
| 9 | ANT | RF (NC on the default U.FL version) |
| 10 | GND | |
| 11 | BUSY | Busy indicator |
| 12 | DIO1 | IRQ line |

## Regulatory

US use is the 902–928 MHz ISM band (915 MHz). Stay within local power and
duty-cycle rules. The product sheet lists FCC ID `Z4T-WIO-SX1262`.
