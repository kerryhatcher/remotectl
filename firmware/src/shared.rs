//! State shared between the hardware loop in `main` and the network tasks.

use core::cell::RefCell;
use core::sync::atomic::{AtomicI32, AtomicU32};

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant};
use heapless::String;

pub use remotectl_shared::{DISPLAY_MESSAGE_MAX as MESSAGE_LEN, KEYBOARD_TEXT_MAX as TYPE_MAX};

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

/// Text for the USB keyboard to type.
pub struct TypeJob {
    pub text: String<TYPE_MAX>,
    /// Pause after each key press and each release.
    pub delay_ms: u16,
    /// Press Command+Return after typing, if true.
    pub command_return: bool,
    /// Raw keyboard report to press and release, instead of text.
    pub chord: Option<(u8, [u8; 6])>,
    pub unicode: Option<(u8, u32)>,
}

/// One job at a time: the API answers 503 while the keyboard is still typing.
pub static TYPE_QUEUE: embassy_sync::channel::Channel<
    CriticalSectionRawMutex,
    Pending<TypeJob>,
    1,
> = embassy_sync::channel::Channel::new();

/// Queue of consumer-page usages for media keys.
pub static MEDIA_QUEUE: embassy_sync::channel::Channel<CriticalSectionRawMutex, Pending<u16>, 4> =
    embassy_sync::channel::Channel::new();

/// Mouse button, as the bit it sets in the HID report.
#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// Action for the USB mouse.
pub enum MouseCmd {
    /// Relative pointer movement in HID counts (the host applies its pointer acceleration).
    Move {
        dx: i16,
        dy: i16,
    },
    Click {
        button: MouseButton,
        double: bool,
    },
    Scroll {
        amount: i8,
    },
}

/// A few queued mouse actions; the API answers 503 when it is full.
pub static MOUSE_QUEUE: embassy_sync::channel::Channel<
    CriticalSectionRawMutex,
    Pending<MouseCmd>,
    4,
> = embassy_sync::channel::Channel::new();

/// True while a USB host has configured the device (keyboard is usable).
pub static USB_CONFIGURED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// USB suspend/disconnect invalidates every earlier input job, even after a quick wake.
pub static USB_SUSPENDED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
pub static USB_EPOCH: AtomicU32 = AtomicU32::new(0);
pub static KEYBOARD_USB_EVENT: Signal<CriticalSectionRawMutex, ()> = Signal::new();
pub static MOUSE_USB_EVENT: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Input admission timestamp and USB session accompany every queued action.
pub struct Pending<T> {
    pub action: T,
    pub epoch: u32,
    pub accepted: Instant,
}

impl<T> Pending<T> {
    pub fn new(action: T, epoch: u32) -> Self {
        Self {
            action,
            epoch,
            accepted: Instant::now(),
        }
    }
    pub fn deadline(&self) -> Instant {
        self.accepted + Duration::from_millis(remotectl_shared::input_expiry::INPUT_TTL_MS)
    }
    pub fn is_current(&self) -> bool {
        match usb_input_epoch() {
            Some(epoch) => remotectl_shared::input_expiry::is_current(
                self.epoch,
                epoch,
                self.accepted.as_millis(),
                Instant::now().as_millis(),
                true,
            ),
            None => false,
        }
    }
}

/// Capture an active USB session. An interrupted snapshot fails closed.
pub fn usb_input_epoch() -> Option<u32> {
    use core::sync::atomic::Ordering::SeqCst;
    let epoch = USB_EPOCH.load(SeqCst);
    if USB_CONFIGURED.load(SeqCst) && !USB_SUSPENDED.load(SeqCst) && epoch == USB_EPOCH.load(SeqCst)
    {
        Some(epoch)
    } else {
        None
    }
}

/// Invalidate active and queued actions whenever USB stops accepting input.
pub fn invalidate_input() {
    use core::sync::atomic::Ordering::SeqCst;
    USB_EPOCH.fetch_add(1, SeqCst);
    TYPE_QUEUE.clear();
    MOUSE_QUEUE.clear();
    MEDIA_QUEUE.clear();
    KEYBOARD_USB_EVENT.signal(());
    MOUSE_USB_EVENT.signal(());
}

/// Wake HID tasks so they can clear any pressed-key state after resume.
pub fn notify_input_resume() {
    KEYBOARD_USB_EVENT.signal(());
    MOUSE_USB_EVENT.signal(());
}

/// Raised by `POST /bootloader` to reboot into the ROM download mode.
pub static BOOTLOADER: Signal<CriticalSectionRawMutex, ()> = Signal::new();
