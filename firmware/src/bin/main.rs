#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use core::cell::RefCell;
use core::fmt::Write;

use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Timer};
use embedded_graphics::mono_font::ascii::{FONT_6X10, FONT_10X20};
use embedded_graphics::mono_font::{MonoTextStyle, MonoTextStyleBuilder};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, PrimitiveStyleBuilder, Rectangle};
use embedded_graphics::text::Text;
use embedded_hal::spi::SpiDevice;
use embedded_hal_bus::spi::RefCellDevice;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig};
use esp_hal::rng::Rng;
use esp_hal::spi::Mode;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use firmware::shared::{DISPLAY_CMD, DisplayCmd, MESSAGE_LEN, RADIO, RadioStatus};
use firmware::{api, wifi};
use heapless::String;
use log::{error, info};
use mipidsi::Builder;
use mipidsi::interface::SpiInterface;
use mipidsi::models::ST7789;
use mipidsi::options::{ColorInversion, Orientation, Rotation};

extern crate alloc;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

// SX1262 opcodes
const OP_GET_STATUS: u8 = 0xC0;
const OP_READ_REGISTER: u8 = 0x1D;
// LoRa sync word register (reset value 0x1424)
const REG_SYNC_WORD_MSB: u16 = 0x0740;

// Adafruit 1.14" 240x135 TFT (ST7789) sits in the middle of the controller's
// 240x320 RAM. Offsets are for the native portrait orientation.
const DISPLAY_WIDTH: u16 = 135;
const DISPLAY_HEIGHT: u16 = 240;
const DISPLAY_OFFSET_X: u16 = 52;
const DISPLAY_OFFSET_Y: u16 = 40;

async fn wait_busy_low(busy: &Input<'_>) -> bool {
    for _ in 0..100 {
        if busy.is_low() {
            return true;
        }
        Timer::after(Duration::from_millis(1)).await;
    }
    false
}

/// Result of one radio check, shown on the display.
enum RadioCheck {
    Ok { status: u8, sync: u16 },
    BadSync { status: u8, sync: u16 },
    BusyStuck,
    SpiError,
}

fn draw_frame<D>(
    display: &mut D,
    check: &RadioCheck,
    tick: u32,
    message: &str,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let size = display.bounding_box().size;

    // Border exactly at the panel edge: if it is clipped or offset, the
    // DISPLAY_OFFSET_* constants need adjusting.
    Rectangle::new(Point::zero(), size)
        .into_styled(PrimitiveStyle::with_stroke(Rgb565::WHITE, 1))
        .draw(display)?;

    let title = MonoTextStyle::new(&FONT_10X20, Rgb565::CYAN);
    Text::new("XIAO", Point::new(10, 28), title).draw(display)?;
    Text::new("ESP32-S3", Point::new(10, 52), title).draw(display)?;

    // Background color overwrites the previous frame's text without clearing.
    let line = |color| {
        MonoTextStyleBuilder::new()
            .font(&FONT_10X20)
            .text_color(color)
            .background_color(Rgb565::BLACK)
            .build()
    };
    let small = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(Rgb565::WHITE)
        .background_color(Rgb565::BLACK)
        .build();

    // Portrait is 135 px wide: 12 chars of the 10x20 font, 20 of the 6x10 font.
    let mut text: String<32> = String::new();
    let (label, color) = match check {
        RadioCheck::Ok { .. } => ("OK          ", Rgb565::GREEN),
        RadioCheck::BadSync { .. } => ("BAD SYNC    ", Rgb565::YELLOW),
        RadioCheck::BusyStuck => ("BUSY HIGH   ", Rgb565::RED),
        RadioCheck::SpiError => ("SPI ERROR   ", Rgb565::RED),
    };
    Text::new("SX1262", Point::new(10, 90), line(Rgb565::WHITE)).draw(display)?;
    Text::new(label, Point::new(10, 114), line(color)).draw(display)?;

    match check {
        RadioCheck::Ok { status, sync } | RadioCheck::BadSync { status, sync } => {
            let _ = write!(text, "status 0x{status:02X}      ");
            Text::new(&text, Point::new(10, 140), small).draw(display)?;
            text.clear();
            let _ = write!(text, "sync   0x{sync:04X}    ");
            Text::new(&text, Point::new(10, 152), small).draw(display)?;
        }
        _ => {
            Text::new("no response        ", Point::new(10, 140), small).draw(display)?;
            Text::new("                   ", Point::new(10, 152), small).draw(display)?;
        }
    }

    text.clear();
    let _ = write!(text, "check #{tick}      ");
    Text::new(&text, Point::new(10, 164), small).draw(display)?;

    // Message from the REST API: up to 3 lines of 20 characters, padded so a
    // shorter message overwrites the previous one.
    for (i, y) in [182, 194, 206].into_iter().enumerate() {
        let chunk = message
            .get(i * 20..)
            .map_or("", |rest| &rest[..rest.len().min(20)]);
        text.clear();
        let _ = write!(text, "{chunk:<20}");
        Text::new(&text, Point::new(10, y), small).draw(display)?;
    }

    // Color bars: red, green, blue. Wrong order or inverted colors show here.
    let bar = |x, color| {
        Rectangle::new(Point::new(x, 216), Size::new(35, 20))
            .into_styled(PrimitiveStyleBuilder::new().fill_color(color).build())
    };
    bar(8, Rgb565::RED).draw(display)?;
    bar(50, Rgb565::GREEN).draw(display)?;
    bar(92, Rgb565::BLUE).draw(display)?;

    Ok(())
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    info!("Embassy initialized!");

    // Wi-Fi + REST API. Credentials and the API token are compiled in.
    let rng = Rng::new();
    let seed = (u64::from(rng.random()) << 32) | u64::from(rng.random());
    let stack = wifi::start(&spawner, peripherals.WIFI, seed);
    for id in 0..api::WEB_TASKS {
        spawner.spawn(api::web_task(id, stack).expect("web task"));
    }

    // Wio-SX1262 on XIAO ESP32S3
    let nss = Output::new(peripherals.GPIO41, Level::High, OutputConfig::default());
    let mut reset = Output::new(peripherals.GPIO42, Level::High, OutputConfig::default());
    let busy = Input::new(peripherals.GPIO40, InputConfig::default());

    // Display: Adafruit 1.14" TFT. CL=D8(SCK), DA=D10(MOSI), CS=D6, DC=D7
    let display_cs = Output::new(peripherals.GPIO43, Level::High, OutputConfig::default());
    let display_dc = Output::new(peripherals.GPIO44, Level::Low, OutputConfig::default());

    // One SPI bus shared by the radio and the display, each with its own CS.
    // 10 MHz is within both the SX1262 (16 MHz max) and the ST7789 limits.
    let spi = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(10))
            .with_mode(Mode::_0),
    )
    .expect("spi config")
    .with_sck(peripherals.GPIO7)
    .with_miso(peripherals.GPIO8)
    .with_mosi(peripherals.GPIO9);
    let spi_bus = RefCell::new(spi);

    let mut radio = RefCellDevice::new(&spi_bus, nss, Delay::new()).expect("radio cs");
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
    display.clear(Rgb565::BLACK).expect("display clear");
    info!("Display initialized");

    let mut tick: u32 = 0;
    let mut message: String<MESSAGE_LEN> = String::new();
    let mut check = RadioCheck::BusyStuck;
    let mut run_check = true;
    loop {
        if run_check {
            tick += 1;

            // Hardware reset
            reset.set_low();
            Timer::after(Duration::from_millis(5)).await;
            reset.set_high();
            Timer::after(Duration::from_millis(10)).await;

            check = if !wait_busy_low(&busy).await {
                error!("BUSY stuck high after reset (check wiring/pins)");
                RadioCheck::BusyStuck
            } else {
                // GetStatus: opcode, then one status byte
                let mut status = [OP_GET_STATUS, 0x00];
                match radio.transfer_in_place(&mut status) {
                    Ok(()) => info!(
                        "GetStatus -> 0x{:02X} (chip mode {}, cmd status {})",
                        status[1],
                        (status[1] >> 4) & 0x7,
                        (status[1] >> 1) & 0x7
                    ),
                    Err(e) => error!("GetStatus SPI error: {:?}", e),
                }

                wait_busy_low(&busy).await;

                // ReadRegister: opcode, addr hi, addr lo, NOP/status, then data bytes
                let [hi, lo] = REG_SYNC_WORD_MSB.to_be_bytes();
                let mut buf = [OP_READ_REGISTER, hi, lo, 0x00, 0x00, 0x00];
                match radio.transfer_in_place(&mut buf) {
                    Ok(()) => {
                        let sync = u16::from_be_bytes([buf[4], buf[5]]);
                        if sync == 0x1424 {
                            info!("Sync word reg = 0x{sync:04X} (expected 0x1424) -> SX1262 OK");
                            RadioCheck::Ok {
                                status: status[1],
                                sync,
                            }
                        } else {
                            error!("Sync word reg = 0x{sync:04X} (expected 0x1424)");
                            RadioCheck::BadSync {
                                status: status[1],
                                sync,
                            }
                        }
                    }
                    Err(e) => {
                        error!("ReadRegister SPI error: {:?}", e);
                        RadioCheck::SpiError
                    }
                }
            };

            RADIO.lock(|cell| {
                *cell.borrow_mut() = match check {
                    RadioCheck::Ok { status, sync } => RadioStatus {
                        ok: true,
                        checks: tick,
                        status_byte: status,
                        sync_word: sync,
                    },
                    RadioCheck::BadSync { status, sync } => RadioStatus {
                        ok: false,
                        checks: tick,
                        status_byte: status,
                        sync_word: sync,
                    },
                    RadioCheck::BusyStuck | RadioCheck::SpiError => RadioStatus {
                        ok: false,
                        checks: tick,
                        status_byte: 0,
                        sync_word: 0,
                    },
                };
            });
        }

        if let Err(e) = draw_frame(&mut display, &check, tick, &message) {
            error!("Display draw error: {:?}", e);
        }

        // Re-check the radio every 2 s, or redraw right away when the API changes the message.
        match select(Timer::after(Duration::from_secs(2)), DISPLAY_CMD.wait()).await {
            Either::First(()) => run_check = true,
            Either::Second(DisplayCmd::Show(text)) => {
                message.clear();
                // The display font is ASCII only.
                for c in text.chars() {
                    let _ = message.push(if c.is_ascii_graphic() || c == ' ' {
                        c
                    } else {
                        '?'
                    });
                }
                info!("Display message: {}", message.as_str());
                run_check = false;
            }
            Either::Second(DisplayCmd::Clear) => {
                message.clear();
                run_check = false;
            }
        }
    }
}
