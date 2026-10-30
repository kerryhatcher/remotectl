//! State shared between the hardware loop in `main` and the network tasks.

use core::cell::RefCell;
use core::sync::atomic::{AtomicI32, AtomicU32};

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use heapless::String;

/// Longest message that can be shown on the display (3 lines of 20 characters).
pub const MESSAGE_LEN: usize = 60;

/// Request from the REST API to change what the display shows.
pub enum DisplayCmd {
    Show(String<MESSAGE_LEN>),
    Clear,
}

/// Latest result of the SX1262 register check.
#[derive(Clone, Copy)]
pub struct RadioStatus {
    pub ok: bool,
    pub checks: u32,
    pub status_byte: u8,
    pub sync_word: u16,
}

pub static DISPLAY_CMD: Signal<CriticalSectionRawMutex, DisplayCmd> = Signal::new();

pub static RADIO: Mutex<CriticalSectionRawMutex, RefCell<RadioStatus>> =
    Mutex::new(RefCell::new(RadioStatus {
        ok: false,
        checks: 0,
        status_byte: 0,
        sync_word: 0,
    }));

/// IPv4 address from DHCP, `None` while not connected.
pub static IP_ADDR: Mutex<CriticalSectionRawMutex, RefCell<Option<[u8; 4]>>> =
    Mutex::new(RefCell::new(None));

/// Last Wi-Fi RSSI in dBm, refreshed every few seconds while connected.
pub static RSSI_DBM: AtomicI32 = AtomicI32::new(0);

/// Number of authorized API requests served since boot.
pub static REQUESTS: AtomicU32 = AtomicU32::new(0);

/// Longest text accepted by one `POST /keyboard/type` request.
pub const TYPE_MAX: usize = 256;

/// Text for the USB keyboard to type.
pub struct TypeJob {
    pub text: String<TYPE_MAX>,
    /// Pause after each key press and each release.
    pub delay_ms: u16,
}

/// One job at a time: the API answers 503 while the keyboard is still typing.
pub static TYPE_QUEUE: embassy_sync::channel::Channel<CriticalSectionRawMutex, TypeJob, 1> =
    embassy_sync::channel::Channel::new();

/// True while a USB host has configured the device (keyboard is usable).
pub static USB_CONFIGURED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Raised by `POST /bootloader` to reboot into the ROM download mode.
pub static BOOTLOADER: Signal<CriticalSectionRawMutex, ()> = Signal::new();
