//! Reboot into the ROM download mode so the board can be reflashed without buttons.

use embassy_time::{Duration, Timer};
use log::info;

use crate::shared::BOOTLOADER;

/// `RTC_CNTL_OPTION1_REG` on the ESP32-S3; bit 0 is `FORCE_DOWNLOAD_BOOT`.
const RTC_CNTL_OPTION1_REG: *mut u32 = 0x6000_812C as *mut u32;
const FORCE_DOWNLOAD_BOOT: u32 = 1;

#[embassy_executor::task]
pub async fn bootloader_task() {
    BOOTLOADER.wait().await;
    info!("Rebooting into the download mode");
    // Let the HTTP response go out first.
    Timer::after(Duration::from_millis(500)).await;

    // SAFETY: a plain write to a documented RTC register; the ROM reads it after the reset.
    unsafe { RTC_CNTL_OPTION1_REG.write_volatile(FORCE_DOWNLOAD_BOOT) };
    esp_hal::system::software_reset()
}
