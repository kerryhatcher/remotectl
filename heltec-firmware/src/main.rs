#![no_std]
#![no_main]

use ed25519_dalek::SigningKey;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer, with_timeout};
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    gpio::{Input, InputConfig, Level, Output, OutputConfig},
    rng::{Trng, TrngSource},
    spi::{
        Mode,
        master::{Config as SpiConfig, Spi},
    },
    time::Rate,
    timer::timg::TimerGroup,
    uart::{Config as UartConfig, Uart},
};
use lora_phy::{
    LoRa,
    iv::GenericSx126xInterfaceVariant,
    sx126x::{Config as RadioConfig, Sx126x, Sx1262, TcxoCtrlVoltage},
};
use remotectl_shared::{parse_key, radio, secure, serial::Decoder};

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    let timer = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timer.timer0, peripherals.FROM_CPU_INTR0);
    let _trng_source = TrngSource::new(peripherals.RNG, peripherals.ADC1);
    let trng = Trng::try_new().expect("TRNG source");
    let mut uart = Uart::new(peripherals.UART0, UartConfig::default())
        .expect("UART initialization")
        .with_tx(peripherals.GPIO43)
        .with_rx(peripherals.GPIO44)
        .into_async();
    let spi = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(8))
            .with_mode(Mode::_0),
    )
    .expect("SPI")
    .with_sck(peripherals.GPIO9)
    .with_mosi(peripherals.GPIO10)
    .with_miso(peripherals.GPIO11);
    let cs = Output::new(peripherals.GPIO8, Level::High, OutputConfig::default());
    let reset = Output::new(peripherals.GPIO12, Level::High, OutputConfig::default());
    let busy = Input::new(peripherals.GPIO13, InputConfig::default());
    let irq = Input::new(peripherals.GPIO14, InputConfig::default());
    let dev = ExclusiveDevice::new(spi, cs, Delay::new()).expect("SPI device");
    let iv = GenericSx126xInterfaceVariant::new(reset, irq, busy, None, None).expect("radio pins");
    let config = RadioConfig {
        chip: Sx1262,
        tcxo_ctrl: Some(TcxoCtrlVoltage::Ctrl1V8),
        use_dcdc: true,
        rx_boost: false,
    };
    let mut lora = LoRa::new(
        Sx126x::new(radio::BlockingSpi(dev), iv, config),
        false,
        embassy_time::Delay,
    )
    .await
    .expect("radio init");
    let key = parse_key(env!("REMOTECTL_LINK_KEY"));
    let signing = parse_key(env!("REMOTECTL_SIGN_KEY")).map(|key| SigningKey::from_bytes(&key));
    let mut decoder = Decoder::new();
    let mut byte = [0u8; 1];
    loop {
        if uart.read_async(&mut byte).await.is_err() {
            continue;
        }
        let frame = match decoder.push(byte[0]) {
            Ok(Some(frame)) => frame,
            Ok(None) => continue,
            Err(_) => {
                let _ = uart.write_async(&[1]).await;
                continue;
            }
        };
        let (Some(key), Some(signing)) = (&key, &signing) else {
            let _ = uart.write_async(&[4]).await;
            continue;
        };
        let mut rx = [0u8; radio::MAX_FRAME];
        if radio::send(&mut lora, radio::HELLO).await.is_err() {
            let _ = uart.write_async(&[3]).await;
            continue;
        }
        let response =
            with_timeout(Duration::from_secs(5), radio::receive(&mut lora, &mut rx)).await;
        let Ok(Ok(len)) = response else {
            let _ = uart.write_async(&[2]).await;
            continue;
        };
        if len != radio::CHALLENGE_PREFIX.len() + radio::CHALLENGE_SIZE
            || !rx[..len].starts_with(radio::CHALLENGE_PREFIX)
        {
            let _ = uart.write_async(&[2]).await;
            continue;
        }
        let mut challenge = [0u8; 16];
        challenge.copy_from_slice(&rx[radio::CHALLENGE_PREFIX.len()..len]);
        let mut nonce = [0u8; 24];
        trng.read(&mut nonce);
        let mut packet = [0u8; secure::MAX_PACKET];
        let Ok(size) = secure::seal(
            key,
            signing,
            &challenge,
            1,
            &nonce,
            &frame[6..],
            &mut packet,
        ) else {
            let _ = uart.write_async(&[1]).await;
            continue;
        };
        Timer::after(Duration::from_millis(200)).await;
        if radio::send(&mut lora, &packet[..size]).await.is_err() {
            let _ = uart.write_async(&[3]).await;
            continue;
        }
        let status =
            match with_timeout(Duration::from_secs(5), radio::receive(&mut lora, &mut rx)).await {
                Ok(Ok(n)) => secure::verify_ack(key, &challenge, &rx[..n]).unwrap_or(2),
                _ => 2,
            };
        let _ = uart.write_async(&[status]).await;
    }
}
