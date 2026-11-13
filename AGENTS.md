# Repository instructions

## Firmware updates and USB recovery

- Every device firmware must provide and preserve a way to trigger a reset into its flashing/bootloader mode with a command over USB, so firmware can be updated without pressing physical buttons. This applies to new firmware, USB changes, and bootloader or flashing changes.
- Provide and document the host-side command or script needed to enter the bootloader and reflash each supported board. A network-only reset endpoint is not a substitute for the USB command.
- When changing firmware or the USB interface, verify that the USB reset and reflashing path still works on the affected hardware when available. Clearly report any unverified behavior or limitations; do not claim that button-free recovery is guaranteed if it has not been demonstrated. If the path is unavailable or broken, treat that as unfinished work and surface it rather than silently relying on physical buttons.
