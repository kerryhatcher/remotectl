#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use core::cell::RefCell;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use embedded_hal_bus::spi::RefCellDevice;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig};
use esp_hal::spi::Mode;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use log::info;
use mipidsi::Builder;
use mipidsi::interface::SpiInterface;
use mipidsi::models::ST7789;
use mipidsi::options::{ColorInversion, Orientation, Rotation};
use xiao_firmware::{system, usb_hid};

extern crate alloc;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

// Adafruit 1.14-inch ST7789 panel, native portrait orientation.
const DISPLAY_WIDTH: u16 = 135;
const DISPLAY_HEIGHT: u16 = 240;
const DISPLAY_OFFSET_X: u16 = 52;
const DISPLAY_OFFSET_Y: u16 = 40;

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // USB CDC is reserved for recovery; logs go to a RAM buffer, see GET /logs.
    xiao_firmware::logger::init(log::LevelFilter::Info);

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    info!("Embassy initialized!");

    // Reserve the ADC entropy source for cryptographic challenges. Wi-Fi is disabled
    // while operating as a LoRa HID receiver.
    let _entropy = esp_hal::rng::TrngSource::new(peripherals.RNG, peripherals.ADC1);
    let trng = esp_hal::rng::Trng::try_new().expect("TRNG source");
    usb_hid::start(
        &spawner,
        peripherals.USB_FS,
        peripherals.GPIO20,
        peripherals.GPIO19,
    );
    spawner.spawn(system::bootloader_task().expect("bootloader task"));

    let nss = Output::new(peripherals.GPIO41, Level::High, OutputConfig::default());
    let reset = Output::new(peripherals.GPIO42, Level::High, OutputConfig::default());
    let busy = Input::new(peripherals.GPIO40, InputConfig::default());
    let irq = Input::new(peripherals.GPIO39, InputConfig::default());
    let rxen = Output::new(peripherals.GPIO38, Level::Low, OutputConfig::default());
    let display_cs = Output::new(peripherals.GPIO43, Level::High, OutputConfig::default());
    let display_dc = Output::new(peripherals.GPIO44, Level::Low, OutputConfig::default());
    let spi = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(10))
            .with_mode(Mode::_0),
    )
    .expect("SPI config")
    .with_sck(peripherals.GPIO7)
    .with_miso(peripherals.GPIO8)
    .with_mosi(peripherals.GPIO9);
    let spi_bus = RefCell::new(spi);
    let radio_dev = RefCellDevice::new(&spi_bus, nss, Delay::new()).expect("radio cs");
    let display_spi = RefCellDevice::new(&spi_bus, display_cs, Delay::new()).expect("display cs");
    let mut display_buf = [0u8; 512];
    let di = SpiInterface::new(display_spi, display_dc, &mut display_buf);
    let mut display = Builder::new(ST7789, di)
        .display_size(DISPLAY_WIDTH, DISPLAY_HEIGHT)
        .display_offset(DISPLAY_OFFSET_X, DISPLAY_OFFSET_Y)
        .orientation(Orientation::new().rotate(Rotation::Deg0))
        .invert_colors(ColorInversion::Inverted)
        .init(&mut Delay::new())
        .expect("display init");
    let _ = display.clear(Rgb565::BLACK);
    let label = MonoTextStyle::new(&FONT_10X20, Rgb565::CYAN);
    let _ = Text::new("LoRa HID", Point::new(10, 28), label).draw(&mut display);
    let _ = Text::new("receiver", Point::new(10, 52), label).draw(&mut display);

    let iv = lora_phy::iv::GenericSx126xInterfaceVariant::new(reset, irq, busy, Some(rxen), None)
        .expect("radio pins");
    let config = lora_phy::sx126x::Config {
        chip: lora_phy::sx126x::Sx1262,
        tcxo_ctrl: Some(lora_phy::sx126x::TcxoCtrlVoltage::Ctrl1V8),
        use_dcdc: true,
        rx_boost: false,
    };
    let mut radio = lora_phy::LoRa::new(
        lora_phy::sx126x::Sx126x::new(remotectl_shared::radio::BlockingSpi(radio_dev), iv, config),
        false,
        embassy_time::Delay,
    )
    .await
    .expect("radio init");
    let key = remotectl_shared::parse_key(env!("REMOTECTL_LINK_KEY"));
    let verifying = remotectl_shared::parse_key(env!("REMOTECTL_VERIFY_KEY"))
        .and_then(|key| ed25519_dalek::VerifyingKey::from_bytes(&key).ok());
    let mut session: Option<remotectl_shared::secure::Receiver> = None;
    let mut active_challenge: Option<[u8; 16]> = None;
    let mut buffer = [0u8; remotectl_shared::radio::MAX_FRAME];
    loop {
        let Ok(len) = remotectl_shared::radio::receive(&mut radio, &mut buffer).await else {
            continue;
        };
        if &buffer[..len] == remotectl_shared::radio::HELLO {
            if key.is_none() || verifying.is_none() {
                continue;
            }
            let mut challenge = [0u8; 16];
            trng.read(&mut challenge);
            session = Some(remotectl_shared::secure::Receiver::new(challenge));
            active_challenge = Some(challenge);
            let prefix = remotectl_shared::radio::CHALLENGE_PREFIX;
            let mut reply = [0u8; 64];
            reply[..prefix.len()].copy_from_slice(prefix);
            reply[prefix.len()..prefix.len() + 16].copy_from_slice(&challenge);
            Timer::after(Duration::from_millis(200)).await;
            let _ = remotectl_shared::radio::send(&mut radio, &reply[..prefix.len() + 16]).await;
            continue;
        }
        let (Some(session), Some(key), Some(verifying)) = (&mut session, &key, &verifying) else {
            continue;
        };
        let mut plaintext = [0u8; 123];
        let Ok(command) = session.open(key, verifying, &buffer[..len], &mut plaintext) else {
            continue;
        };
        use remotectl_shared::{InputCommand, MouseButton};
        use xiao_firmware::shared::{
            MOUSE_QUEUE, MouseButton as HidButton, MouseCmd, Pending, TYPE_QUEUE, TypeJob,
            usb_input_epoch,
        };
        let epoch = usb_input_epoch();
        let status = match command {
            InputCommand::Ping => 0,
            _ if epoch.is_none() => {
                if xiao_firmware::shared::USB_SUSPENDED.load(core::sync::atomic::Ordering::SeqCst) {
                    8
                } else {
                    5
                }
            }
            InputCommand::Type { text, delay_ms }
            | InputCommand::TypeCommandReturn { text, delay_ms } => {
                let command_return = matches!(command, InputCommand::TypeCommandReturn { .. });
                match heapless::String::try_from(text) {
                    Ok(text) => {
                        if TYPE_QUEUE
                            .try_send(Pending::new(
                                TypeJob {
                                    text,
                                    delay_ms,
                                    command_return,
                                    chord: None,
                                    unicode: None,
                                },
                                epoch.unwrap(),
                            ))
                            .is_ok()
                        {
                            0
                        } else {
                            6
                        }
                    }
                    Err(_) => 6,
                }
            }
            InputCommand::KeyChord { modifiers, keys } => {
                if TYPE_QUEUE
                    .try_send(Pending::new(
                        TypeJob {
                            text: heapless::String::new(),
                            delay_ms: 15,
                            command_return: false,
                            chord: Some((modifiers, keys)),
                            unicode: None,
                        },
                        epoch.unwrap(),
                    ))
                    .is_ok()
                {
                    0
                } else {
                    6
                }
            }
            InputCommand::Unicode { host, scalar } => {
                if TYPE_QUEUE
                    .try_send(Pending::new(
                        TypeJob {
                            text: heapless::String::new(),
                            delay_ms: 15,
                            command_return: false,
                            chord: None,
                            unicode: Some((host, scalar)),
                        },
                        epoch.unwrap(),
                    ))
                    .is_ok()
                {
                    0
                } else {
                    6
                }
            }
            InputCommand::Media { usage } => {
                if xiao_firmware::shared::MEDIA_QUEUE
                    .try_send(Pending::new(usage, epoch.unwrap()))
                    .is_ok()
                {
                    0
                } else {
                    6
                }
            }
            InputCommand::Move { dx, dy } => {
                if MOUSE_QUEUE
                    .try_send(Pending::new(MouseCmd::Move { dx, dy }, epoch.unwrap()))
                    .is_ok()
                {
                    0
                } else {
                    6
                }
            }
            InputCommand::Click { button, double } => {
                let button = match button {
                    MouseButton::Left => HidButton::Left,
                    MouseButton::Right => HidButton::Right,
                    MouseButton::Middle => HidButton::Middle,
                };
                if MOUSE_QUEUE
                    .try_send(Pending::new(
                        MouseCmd::Click { button, double },
                        epoch.unwrap(),
                    ))
                    .is_ok()
                {
                    0
                } else {
                    6
                }
            }
            InputCommand::Scroll { amount } => {
                if MOUSE_QUEUE
                    .try_send(Pending::new(MouseCmd::Scroll { amount }, epoch.unwrap()))
                    .is_ok()
                {
                    0
                } else {
                    6
                }
            }
        };
        if let Some(challenge) = active_challenge {
            let ack = remotectl_shared::secure::make_ack(key, &challenge, status);
            Timer::after(Duration::from_millis(200)).await;
            let _ = remotectl_shared::radio::send(&mut radio, &ack).await;
        }
    }
}
