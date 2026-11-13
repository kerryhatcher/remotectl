//! USB HID keyboard and mouse on the ESP32-S3's native USB (full-speed OTG).
//!
//! GPIO19 (D-) and GPIO20 (D+) are shared with the USB-Serial-JTAG block.
//! The composite device exposes a CDC recovery interface in addition to HID.
//! Sending `BOOTLOADER\n` over CDC requests ROM download mode.

use core::sync::atomic::Ordering;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embassy_usb::class::cdc_acm::{CdcAcmClass, State as CdcState};
use embassy_usb::class::hid::{
    Config as HidConfig, HidBootProtocol, HidSubclass, HidWriter, State,
};
use embassy_usb::{Builder, Config, Handler, UsbDevice};
use esp_hal::peripherals::{GPIO19, GPIO20, USB_FS};
use esp_hal::usb::otg::Usb;
use esp_hal::usb::otg::embassy_usb_device::{Config as DriverConfig, Driver};
use log::{info, warn};
use static_cell::StaticCell;

use crate::shared::{BOOTLOADER, MOUSE_QUEUE, MouseButton, MouseCmd, TYPE_QUEUE, USB_CONFIGURED};

const SHIFT: u8 = 0x02;

/// Standard boot-protocol keyboard: 8 modifier bits, a reserved byte, 5 LED outputs and 6 key slots.
const KEYBOARD_REPORT_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, // Usage Page (Generic Desktop)
    0x09, 0x06, // Usage (Keyboard)
    0xA1, 0x01, // Collection (Application)
    0x05, 0x07, //   Usage Page (Keyboard)
    0x19, 0xE0, //   Usage Minimum (Left Control)
    0x29, 0xE7, //   Usage Maximum (Right GUI)
    0x15, 0x00, //   Logical Minimum (0)
    0x25, 0x01, //   Logical Maximum (1)
    0x75, 0x01, //   Report Size (1)
    0x95, 0x08, //   Report Count (8)
    0x81, 0x02, //   Input (Data, Variable, Absolute): modifiers
    0x95, 0x01, //   Report Count (1)
    0x75, 0x08, //   Report Size (8)
    0x81, 0x01, //   Input (Constant): reserved byte
    0x95, 0x05, //   Report Count (5)
    0x75, 0x01, //   Report Size (1)
    0x05, 0x08, //   Usage Page (LEDs)
    0x19, 0x01, //   Usage Minimum (Num Lock)
    0x29, 0x05, //   Usage Maximum (Kana)
    0x91, 0x02, //   Output (Data, Variable, Absolute): LEDs
    0x95, 0x01, //   Report Count (1)
    0x75, 0x03, //   Report Size (3)
    0x91, 0x01, //   Output (Constant): LED padding
    0x95, 0x06, //   Report Count (6)
    0x75, 0x08, //   Report Size (8)
    0x15, 0x00, //   Logical Minimum (0)
    0x25, 0x65, //   Logical Maximum (101)
    0x05, 0x07, //   Usage Page (Keyboard)
    0x19, 0x00, //   Usage Minimum (0)
    0x29, 0x65, //   Usage Maximum (101)
    0x81, 0x00, //   Input (Data, Array): key codes
    0xC0, // End Collection
];

/// Boot-protocol mouse: 3 buttons, relative X and Y, and a wheel (4 bytes).
const MOUSE_REPORT_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, // Usage Page (Generic Desktop)
    0x09, 0x02, // Usage (Mouse)
    0xA1, 0x01, // Collection (Application)
    0x09, 0x01, //   Usage (Pointer)
    0xA1, 0x00, //   Collection (Physical)
    0x05, 0x09, //     Usage Page (Buttons)
    0x19, 0x01, //     Usage Minimum (Button 1)
    0x29, 0x03, //     Usage Maximum (Button 3)
    0x15, 0x00, //     Logical Minimum (0)
    0x25, 0x01, //     Logical Maximum (1)
    0x95, 0x03, //     Report Count (3)
    0x75, 0x01, //     Report Size (1)
    0x81, 0x02, //     Input (Data, Variable, Absolute): buttons
    0x95, 0x01, //     Report Count (1)
    0x75, 0x05, //     Report Size (5)
    0x81, 0x01, //     Input (Constant): padding
    0x05, 0x01, //     Usage Page (Generic Desktop)
    0x09, 0x30, //     Usage (X)
    0x09, 0x31, //     Usage (Y)
    0x09, 0x38, //     Usage (Wheel)
    0x15, 0x81, //     Logical Minimum (-127)
    0x25, 0x7F, //     Logical Maximum (127)
    0x75, 0x08, //     Report Size (8)
    0x95, 0x03, //     Report Count (3)
    0x81, 0x06, //     Input (Data, Variable, Relative): X, Y, wheel
    0xC0, //   End Collection
    0xC0, // End Collection
];

/// Modifier byte and usage code for a character on a US keyboard layout.
fn key_for(c: char) -> Option<(u8, u8)> {
    Some(match c {
        'a'..='z' => (0, 0x04 + (c as u8 - b'a')),
        'A'..='Z' => (SHIFT, 0x04 + (c as u8 - b'A')),
        '1'..='9' => (0, 0x1E + (c as u8 - b'1')),
        '0' => (0, 0x27),
        '!' => (SHIFT, 0x1E),
        '@' => (SHIFT, 0x1F),
        '#' => (SHIFT, 0x20),
        '$' => (SHIFT, 0x21),
        '%' => (SHIFT, 0x22),
        '^' => (SHIFT, 0x23),
        '&' => (SHIFT, 0x24),
        '*' => (SHIFT, 0x25),
        '(' => (SHIFT, 0x26),
        ')' => (SHIFT, 0x27),
        '\n' => (0, 0x28),
        '\t' => (0, 0x2B),
        ' ' => (0, 0x2C),
        '-' => (0, 0x2D),
        '_' => (SHIFT, 0x2D),
        '=' => (0, 0x2E),
        '+' => (SHIFT, 0x2E),
        '[' => (0, 0x2F),
        '{' => (SHIFT, 0x2F),
        ']' => (0, 0x30),
        '}' => (SHIFT, 0x30),
        '\\' => (0, 0x31),
        '|' => (SHIFT, 0x31),
        ';' => (0, 0x33),
        ':' => (SHIFT, 0x33),
        '\'' => (0, 0x34),
        '"' => (SHIFT, 0x34),
        '`' => (0, 0x35),
        '~' => (SHIFT, 0x35),
        ',' => (0, 0x36),
        '<' => (SHIFT, 0x36),
        '.' => (0, 0x37),
        '>' => (SHIFT, 0x37),
        '/' => (0, 0x38),
        '?' => (SHIFT, 0x38),
        _ => return None,
    })
}

/// Tracks whether the host has configured the device.
struct ConnectionHandler;

impl Handler for ConnectionHandler {
    fn configured(&mut self, configured: bool) {
        USB_CONFIGURED.store(configured, Ordering::Relaxed);
        info!(
            "USB host {}",
            if configured {
                "connected"
            } else {
                "disconnected"
            }
        );
    }

    fn reset(&mut self) {
        USB_CONFIGURED.store(false, Ordering::Relaxed);
    }

    fn suspended(&mut self, suspended: bool) {
        info!("USB {}", if suspended { "suspended" } else { "resumed" });
    }
}

static EP_OUT_BUFFER: StaticCell<[u8; 256]> = StaticCell::new();
static CONFIG_DESCRIPTOR: StaticCell<[u8; 512]> = StaticCell::new();
static BOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
static MSOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
static CONTROL_BUFFER: StaticCell<[u8; 64]> = StaticCell::new();
static HID_STATE: StaticCell<State<'static>> = StaticCell::new();
static MOUSE_STATE: StaticCell<State<'static>> = StaticCell::new();
static HANDLER: StaticCell<ConnectionHandler> = StaticCell::new();
static CDC_STATE: StaticCell<CdcState<'static>> = StaticCell::new();

/// Start the USB device and the task that types queued text.
pub fn start(spawner: &Spawner, usb: USB_FS<'static>, dp: GPIO20<'static>, dm: GPIO19<'static>) {
    let usb = Usb::new_fs(usb, dp, dm);
    let driver = Driver::new(usb, EP_OUT_BUFFER.init([0; 256]), DriverConfig::default());

    let mut config = Config::new(0x1209, 0x0001); // pid.codes test VID/PID
    config.manufacturer = Some("remotectl");
    config.product = Some("remotectl hid");
    config.serial_number = Some("0001");
    config.max_power = 100;
    config.max_packet_size_0 = 64;

    let mut builder = Builder::new(
        driver,
        config,
        CONFIG_DESCRIPTOR.init([0; 512]),
        BOS_DESCRIPTOR.init([0; 256]),
        MSOS_DESCRIPTOR.init([0; 256]),
        CONTROL_BUFFER.init([0; 64]),
    );
    builder.handler(HANDLER.init(ConnectionHandler));

    let writer = HidWriter::<_, 8>::new(
        &mut builder,
        HID_STATE.init(State::new()),
        HidConfig {
            report_descriptor: KEYBOARD_REPORT_DESCRIPTOR,
            request_handler: None,
            poll_ms: 10,
            max_packet_size: 8,
            hid_subclass: HidSubclass::Boot,
            hid_boot_protocol: HidBootProtocol::Keyboard,
        },
    );

    let mouse_writer = HidWriter::<_, 4>::new(
        &mut builder,
        MOUSE_STATE.init(State::new()),
        HidConfig {
            report_descriptor: MOUSE_REPORT_DESCRIPTOR,
            request_handler: None,
            poll_ms: 10,
            max_packet_size: 4,
            hid_subclass: HidSubclass::Boot,
            hid_boot_protocol: HidBootProtocol::Mouse,
        },
    );

    let cdc = CdcAcmClass::new(&mut builder, CDC_STATE.init(CdcState::new()), 64);
    spawner.spawn(usb_task(builder.build()).expect("usb task"));
    spawner.spawn(recovery_task(cdc).expect("USB recovery task"));
    spawner.spawn(keyboard_task(writer).expect("keyboard task"));
    spawner.spawn(mouse_task(mouse_writer).expect("mouse task"));
}

#[embassy_executor::task]
async fn usb_task(mut device: UsbDevice<'static, Driver<'static>>) -> ! {
    device.run().await
}

#[embassy_executor::task]
async fn keyboard_task(mut writer: HidWriter<'static, Driver<'static>, 8>) {
    loop {
        let job = TYPE_QUEUE.receive().await;
        let pause = Duration::from_millis(u64::from(job.delay_ms));
        info!("Typing {} characters", job.text.chars().count());

        writer.ready().await;
        let mut skipped = 0;
        for c in job.text.chars() {
            if c == '\r' {
                continue; // CRLF: the \n types the Enter key
            }
            let Some((modifier, usage)) = key_for(c) else {
                skipped += 1;
                continue;
            };
            if writer
                .write(&[modifier, 0, usage, 0, 0, 0, 0, 0])
                .await
                .is_err()
            {
                warn!("USB write failed, dropping the rest of the text");
                break;
            }
            Timer::after(pause).await;
            if writer.write(&[0; 8]).await.is_err() {
                break;
            }
            Timer::after(pause).await;
        }
        let _ = writer.write(&[0; 8]).await; // make sure no key stays pressed
        if skipped > 0 {
            warn!("Skipped {skipped} characters with no US keyboard mapping");
        }
    }
}

/// A host connected over USB can request the ROM bootloader without Wi-Fi.
#[embassy_executor::task]
async fn recovery_task(mut cdc: CdcAcmClass<'static, Driver<'static>>) {
    const COMMAND: &[u8] = b"BOOTLOADER\n";
    let mut matched = 0;
    let mut buffer = [0u8; 64];
    loop {
        cdc.wait_connection().await;
        while let Ok(n) = cdc.read_packet(&mut buffer).await {
            for byte in &buffer[..n] {
                if *byte == COMMAND[matched] {
                    matched += 1;
                    if matched == COMMAND.len() {
                        BOOTLOADER.signal(());
                        matched = 0;
                    }
                } else {
                    matched = usize::from(*byte == COMMAND[0]);
                }
            }
        }
        matched = 0;
    }
}

/// Largest movement one report can carry per axis.
const MOUSE_STEP: i16 = 127;

/// Time between mouse reports, matching the 10 ms endpoint poll interval.
const MOUSE_PAUSE: Duration = Duration::from_millis(10);

async fn mouse_report(
    writer: &mut HidWriter<'static, Driver<'static>, 4>,
    buttons: u8,
    dx: i8,
    dy: i8,
    wheel: i8,
) -> bool {
    writer
        .write(&[buttons, dx as u8, dy as u8, wheel as u8])
        .await
        .is_ok()
}

async fn mouse_click(writer: &mut HidWriter<'static, Driver<'static>, 4>, button: u8) -> bool {
    mouse_report(writer, button, 0, 0, 0).await && {
        Timer::after(Duration::from_millis(40)).await;
        mouse_report(writer, 0, 0, 0, 0).await
    }
}

#[embassy_executor::task]
async fn mouse_task(mut writer: HidWriter<'static, Driver<'static>, 4>) {
    loop {
        let cmd = MOUSE_QUEUE.receive().await;
        writer.ready().await;
        let ok = match cmd {
            MouseCmd::Move { mut dx, mut dy } => {
                let mut ok = true;
                while ok && (dx != 0 || dy != 0) {
                    let (sx, sy) = (
                        dx.clamp(-MOUSE_STEP, MOUSE_STEP),
                        dy.clamp(-MOUSE_STEP, MOUSE_STEP),
                    );
                    ok = mouse_report(&mut writer, 0, sx as i8, sy as i8, 0).await;
                    dx -= sx;
                    dy -= sy;
                    Timer::after(MOUSE_PAUSE).await;
                }
                ok
            }
            MouseCmd::Click { button, double } => {
                let bit = match button {
                    MouseButton::Left => 0x01,
                    MouseButton::Right => 0x02,
                    MouseButton::Middle => 0x04,
                };
                let mut ok = mouse_click(&mut writer, bit).await;
                if ok && double {
                    Timer::after(Duration::from_millis(60)).await;
                    ok = mouse_click(&mut writer, bit).await;
                }
                ok
            }
            MouseCmd::Scroll { amount } => mouse_report(&mut writer, 0, 0, 0, amount).await,
        };
        if !ok {
            warn!("USB mouse write failed, dropping the action");
        }
    }
}
