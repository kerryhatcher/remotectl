# Hardware docs

Device: **Seeed Studio XIAO ESP32S3 & Wio-SX1262 Meshtastic LoRa Dev Kit** (Amazon listing
<https://www.amazon.com/dp/B0F8TSW1WZ>, sold pre-flashed with Meshtastic).

| File | Contents |
|------|----------|
| [hardware.md](hardware.md) | Specs, buttons/LEDs, pin map (XIAO + Wio-SX1262 + SX1262 control pins) |
| [wio-sx1262-module.md](wio-sx1262-module.md) | SX1262 module facts that matter for firmware (RF switch, TCXO, limits) |
| [flashing.md](flashing.md) | Entering the bootloader, `espflash` usage, restoring Meshtastic |
| `pdfs/` | Vendor PDFs (see below) |
| `images/` | Vendor diagrams (button locations, pin lists) |

## PDFs (`pdfs/`)

| File | Source |
|------|--------|
| `Wio-SX1262_Module_Datasheet.pdf` | <https://files.seeedstudio.com/products/SenseCAP/Wio_SX1262/Wio-SX1262_Module_Datasheet.pdf> |
| `Wio-SX1262_for_XIAO_Schematic.pdf` | <https://files.seeedstudio.com/products/SenseCAP/Wio_SX1262/Schematic_Diagram_Wio-SX1262_for_XIAO.pdf> |
| `113010003_Wio-SX1262_product_sheet.pdf` | <https://files.seeedstudio.com/Bazaar/product_pdf/113010003.pdf> |
| `XIAO_ESP32S3_v1.4_Schematic.pdf` | <https://files.seeedstudio.com/wiki/SeeedStudio-XIAO-ESP32S3/new-res/202003751_XIAO%20ESP32S3_v1.4_SCH_260226.pdf.pdf> |
| `ESP32-S3_Datasheet.pdf` | <https://files.seeedstudio.com/wiki/SeeedStudio-XIAO-ESP32S3/res/esp32-s3_datasheet.pdf> |

## Online references

- Kit wiki: <https://wiki.seeedstudio.com/wio_sx1262_with_xiao_esp32s3_kit/>
- XIAO ESP32S3 wiki (buttons, bootloader mode, pin table): <https://wiki.seeedstudio.com/xiao_esp32s3_getting_started/>
- Meshtastic variant (source of the verified LoRa pin map):
  <https://github.com/meshtastic/firmware/blob/master/variants/esp32s3/seeed_xiao_s3/variant.h>
