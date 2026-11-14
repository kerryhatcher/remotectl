//! USB HID keyboard and mouse on the ESP32-S3's native USB (full-speed OTG).
//!
//! GPIO19 (D-) and GPIO20 (D+) are shared with the USB-Serial-JTAG block.
//! The composite device exposes a CDC recovery interface in addition to HID.
//! Sending `BOOTLOADER\n` over CDC requests ROM download mode.

use core::sync::atomic::Ordering;

use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Timer, with_timeout};
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

use crate::shared::{
    BOOTLOADER, KEYBOARD_USB_EVENT, MEDIA_QUEUE, MOUSE_QUEUE, MOUSE_USB_EVENT, MouseButton,
    MouseCmd, Pending, TYPE_QUEUE, TypeJob, USB_CONFIGURED, USB_SUSPENDED, invalidate_input,
    notify_input_resume, usb_input_epoch,
};

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
    0x26, 0xFF, 0x00, //   Logical Maximum (255)
    0x05, 0x07, //   Usage Page (Keyboard)
    0x19, 0x00, //   Usage Minimum (0)
    0x2A, 0xFF, 0x00, //   Usage Maximum (255)
    0x81, 0x00, //   Input (Data, Array): key codes
    0xC0, // End Collection
];

/// Boot-protocol mouse: 3 buttons, relative X and Y, and a wheel (4 bytes).
const MOUSE_REPORT_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, // Usage Page (Generic Desktop)
    0x09, 0x02, // Usage (Mouse)
    0xA1, 0x01, // Collection (Application)
    0x85, 0x01, // Report ID 1: mouse
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
    0x05, 0x0C, 0x09, 0x01, 0xA1, 0x01, // Consumer Control application
    0x85, 0x02, // Report ID 2: consumer key
    0x15, 0x00, 0x26, 0xFF, 0x03, // Logical range 0..1023
    0x19, 0x00, 0x2A, 0xFF, 0x03, // Usage range 0..1023
    0x75, 0x10, 0x95, 0x01, 0x81, 0x00, // One 16-bit array entry
    0xC0,
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
        USB_CONFIGURED.store(configured, Ordering::SeqCst);
        if configured {
            USB_SUSPENDED.store(false, Ordering::SeqCst);
            notify_input_resume();
        } else {
            invalidate_input();
        }
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
        USB_CONFIGURED.store(false, Ordering::SeqCst);
        invalidate_input();
    }

    fn suspended(&mut self, suspended: bool) {
        USB_SUSPENDED.store(suspended, Ordering::SeqCst);
        if suspended {
            invalidate_input();
        } else {
            notify_input_resume();
        }
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

    let mouse_writer = HidWriter::<_, 5>::new(
        &mut builder,
        MOUSE_STATE.init(State::new()),
        HidConfig {
            report_descriptor: MOUSE_REPORT_DESCRIPTOR,
            request_handler: None,
            poll_ms: 10,
            max_packet_size: 5,
            hid_subclass: HidSubclass::No,
            hid_boot_protocol: HidBootProtocol::None,
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

/// Clear a potentially pressed key without allowing a sleeping host to block us.
async fn keyboard_neutral(writer: &mut HidWriter<'static, Driver<'static>, 8>) {
    if usb_input_epoch().is_some() {
        let _ = with_timeout(Duration::from_millis(100), async {
            writer.ready().await;
            if usb_input_epoch().is_some() {
                let _ = writer.write(&[0; 8]).await;
            }
        })
        .await;
    }
}

#[embassy_executor::task]
async fn keyboard_task(mut writer: HidWriter<'static, Driver<'static>, 8>) {
    loop {
        let pending = match select(TYPE_QUEUE.receive(), KEYBOARD_USB_EVENT.wait()).await {
            Either::First(pending) => pending,
            Either::Second(()) => {
                keyboard_neutral(&mut writer).await;
                continue;
            }
        };
        KEYBOARD_USB_EVENT.reset();
        if !pending.is_current() {
            keyboard_neutral(&mut writer).await;
            continue;
        }
        let deadline = pending.deadline();
        keyboard_neutral(&mut writer).await;
        if !pending.is_current() {
            keyboard_neutral(&mut writer).await;
            continue;
        }
        let completed = remotectl_shared::input_expiry::run_until_invalid(
            KEYBOARD_USB_EVENT.wait(),
            Timer::at(deadline),
            execute_keyboard(&mut writer, pending.action),
        )
        .await;
        if !completed {
            warn!("Keyboard action expired or USB session changed");
            keyboard_neutral(&mut writer).await;
        }
    }
}

async fn execute_keyboard(writer: &mut HidWriter<'static, Driver<'static>, 8>, job: TypeJob) {
    let pause = Duration::from_millis(u64::from(job.delay_ms));
    info!("Typing {} characters", job.text.chars().count());

    writer.ready().await;
    if let Some((host, scalar)) = job.unicode {
        // Linux: Ctrl+Shift+U, hexadecimal scalar, Enter. macOS: hold Option
        // while typing exactly four hex digits with Unicode Hex Input active.
        let mut digits = [0u8; 8];
        let mut value = scalar;
        let mut start = digits.len();
        loop {
            start -= 1;
            let digit = (value & 15) as u8;
            digits[start] = if digit < 10 {
                b'0' + digit
            } else {
                b'a' + digit - 10
            };
            value >>= 4;
            if value == 0 && (host == 1 || digits.len() - start >= 4) {
                break;
            }
        }
        if host == 1 {
            let _ = writer.write(&[0x03, 0, 0x18, 0, 0, 0, 0, 0]).await;
            Timer::after(pause).await;
            let _ = writer.write(&[0; 8]).await;
            Timer::after(pause).await;
        }
        let modifier = if host == 2 { 0x04 } else { 0 };
        for digit in &digits[start..] {
            let (_, usage) = key_for(*digit as char).expect("hex digit mapping");
            if writer
                .write(&[modifier, 0, usage, 0, 0, 0, 0, 0])
                .await
                .is_err()
            {
                break;
            }
            Timer::after(pause).await;
            if writer
                .write(&[modifier, 0, 0, 0, 0, 0, 0, 0])
                .await
                .is_err()
            {
                break;
            }
            Timer::after(pause).await;
        }
        if host == 1 {
            let _ = writer.write(&[0, 0, 0x28, 0, 0, 0, 0, 0]).await;
            Timer::after(pause).await;
        }
        let _ = writer.write(&[0; 8]).await;
        return;
    }
    if let Some((modifiers, keys)) = job.chord {
        let mut report = [0u8; 8];
        report[0] = modifiers;
        report[2..].copy_from_slice(&keys);
        if writer.write(&report).await.is_ok() {
            Timer::after(pause).await;
        }
        let _ = writer.write(&[0; 8]).await;
        return;
    }
    let mut skipped = 0;
    let mut write_failed = false;
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
            write_failed = true;
            break;
        }
        Timer::after(pause).await;
        if writer.write(&[0; 8]).await.is_err() {
            write_failed = true;
            break;
        }
        Timer::after(pause).await;
    }
    if job.command_return && !write_failed && skipped == 0 {
        // Left GUI (Command on macOS) + Enter, followed by a full release.
        if writer.write(&[0x08, 0, 0x28, 0, 0, 0, 0, 0]).await.is_ok() {
            Timer::after(pause).await;
        }
    }
    let _ = writer.write(&[0; 8]).await; // make sure no key stays pressed
    if skipped > 0 {
        warn!("Skipped {skipped} characters with no US keyboard mapping");
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
    writer: &mut HidWriter<'static, Driver<'static>, 5>,
    buttons: u8,
    dx: i8,
    dy: i8,
    wheel: i8,
) -> bool {
    writer
        .write(&[1, buttons, dx as u8, dy as u8, wheel as u8])
        .await
        .is_ok()
}

async fn mouse_click(writer: &mut HidWriter<'static, Driver<'static>, 5>, button: u8) -> bool {
    mouse_report(writer, button, 0, 0, 0).await && {
        Timer::after(Duration::from_millis(40)).await;
        mouse_report(writer, 0, 0, 0, 0).await
    }
}

enum PointerAction {
    Mouse(MouseCmd),
    Media(u16),
}

async fn mouse_neutral(writer: &mut HidWriter<'static, Driver<'static>, 5>) {
    if usb_input_epoch().is_some() {
        let _ = with_timeout(Duration::from_millis(100), async {
            writer.ready().await;
            if usb_input_epoch().is_some() {
                let _ = writer.write(&[1, 0, 0, 0, 0]).await;
                let _ = writer.write(&[2, 0, 0]).await;
            }
        })
        .await;
    }
}

#[embassy_executor::task]
async fn mouse_task(mut writer: HidWriter<'static, Driver<'static>, 5>) {
    loop {
        let pending = match select(
            select(MOUSE_QUEUE.receive(), MEDIA_QUEUE.receive()),
            MOUSE_USB_EVENT.wait(),
        )
        .await
        {
            Either::First(Either::First(job)) => Pending {
                action: PointerAction::Mouse(job.action),
                epoch: job.epoch,
                accepted: job.accepted,
            },
            Either::First(Either::Second(job)) => Pending {
                action: PointerAction::Media(job.action),
                epoch: job.epoch,
                accepted: job.accepted,
            },
            Either::Second(()) => {
                mouse_neutral(&mut writer).await;
                continue;
            }
        };
        MOUSE_USB_EVENT.reset();
        if !pending.is_current() {
            mouse_neutral(&mut writer).await;
            continue;
        }
        let deadline = pending.deadline();
        mouse_neutral(&mut writer).await;
        if !pending.is_current() {
            mouse_neutral(&mut writer).await;
            continue;
        }
        let completed = remotectl_shared::input_expiry::run_until_invalid(
            MOUSE_USB_EVENT.wait(),
            Timer::at(deadline),
            execute_pointer(&mut writer, pending.action),
        )
        .await;
        if !completed {
            warn!("Mouse/media action expired or USB session changed");
            mouse_neutral(&mut writer).await;
        }
    }
}

async fn execute_pointer(
    writer: &mut HidWriter<'static, Driver<'static>, 5>,
    action: PointerAction,
) {
    writer.ready().await;
    let cmd = match action {
        PointerAction::Mouse(cmd) => cmd,
        PointerAction::Media(usage) => {
            let [lo, hi] = usage.to_le_bytes();
            if writer.write(&[2, lo, hi]).await.is_ok() {
                Timer::after(Duration::from_millis(20)).await;
            }
            let _ = writer.write(&[2, 0, 0]).await;
            return;
        }
    };
    let ok = match cmd {
        MouseCmd::Move { mut dx, mut dy } => {
            let mut ok = true;
            while ok && (dx != 0 || dy != 0) {
                let (sx, sy) = (
                    dx.clamp(-MOUSE_STEP, MOUSE_STEP),
                    dy.clamp(-MOUSE_STEP, MOUSE_STEP),
                );
                ok = mouse_report(writer, 0, sx as i8, sy as i8, 0).await;
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
            let mut ok = mouse_click(writer, bit).await;
            if ok && double {
                Timer::after(Duration::from_millis(60)).await;
                ok = mouse_click(writer, bit).await;
            }
            ok
        }
        MouseCmd::Scroll { amount } => mouse_report(writer, 0, 0, 0, amount).await,
    };
    if !ok {
        warn!("USB mouse write failed, dropping the action");
    }
}
