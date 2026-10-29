#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig};
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::spi::Mode;
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use log::{error, info};

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

// SX1262 opcodes
const OP_GET_STATUS: u8 = 0xC0;
const OP_READ_REGISTER: u8 = 0x1D;
// LoRa sync word register (reset value 0x1424)
const REG_SYNC_WORD_MSB: u16 = 0x0740;

async fn wait_busy_low(busy: &Input<'_>) -> bool {
    for _ in 0..100 {
        if busy.is_low() {
            return true;
        }
        Timer::after(Duration::from_millis(1)).await;
    }
    false
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    info!("Embassy initialized!");

    // Wio-SX1262 on XIAO ESP32S3
    let mut nss = Output::new(peripherals.GPIO41, Level::High, OutputConfig::default());
    let mut reset = Output::new(peripherals.GPIO42, Level::High, OutputConfig::default());
    let busy = Input::new(peripherals.GPIO40, InputConfig::default());

    let mut spi = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_khz(1000))
            .with_mode(Mode::_0),
    )
    .expect("spi config")
    .with_sck(peripherals.GPIO7)
    .with_miso(peripherals.GPIO8)
    .with_mosi(peripherals.GPIO9);

    loop {
        // Hardware reset
        reset.set_low();
        Timer::after(Duration::from_millis(5)).await;
        reset.set_high();
        Timer::after(Duration::from_millis(10)).await;

        if !wait_busy_low(&busy).await {
            error!("BUSY stuck high after reset (check wiring/pins)");
            Timer::after(Duration::from_secs(2)).await;
            continue;
        }

        // GetStatus: opcode, then one status byte
        let mut status = [OP_GET_STATUS, 0x00];
        nss.set_low();
        let r = spi.transfer(&mut status);
        nss.set_high();
        match r {
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
        nss.set_low();
        let r = spi.transfer(&mut buf);
        nss.set_high();
        match r {
            Ok(()) => {
                let sync = u16::from_be_bytes([buf[4], buf[5]]);
                if sync == 0x1424 {
                    info!("Sync word reg = 0x{:04X} (expected 0x1424) -> SX1262 OK", sync);
                } else {
                    error!("Sync word reg = 0x{:04X} (expected 0x1424)", sync);
                }
            }
            Err(e) => error!("ReadRegister SPI error: {:?}", e),
        }

        Timer::after(Duration::from_secs(2)).await;
    }
}
