//! Common radio transport helpers. Board pin assignments remain in the firmware crates.

use embedded_hal::spi::{ErrorType, Operation};
use embedded_hal_async::{delay::DelayNs, spi::SpiDevice};
use lora_phy::{
    LoRa,
    mod_params::{Bandwidth, CodingRate, RadioError, RxMode, SpreadingFactor},
    mod_traits::RadioKind,
};

/// Adapts a blocking SPI device to the async driver. Each SPI transaction completes
/// synchronously, so a shared bus is held only during the transaction itself.
pub struct BlockingSpi<T>(pub T);
impl<T: ErrorType> ErrorType for BlockingSpi<T> {
    type Error = T::Error;
}
impl<T: embedded_hal::spi::SpiDevice<u8>> SpiDevice<u8> for BlockingSpi<T> {
    async fn transaction(
        &mut self,
        operations: &mut [Operation<'_, u8>],
    ) -> Result<(), Self::Error> {
        self.0.transaction(operations)
    }
}

pub const FREQUENCY_HZ: u32 = 915_000_000;
pub const MAX_FRAME: usize = 255;

pub async fn send<R: RadioKind, D: DelayNs>(
    radio: &mut LoRa<R, D>,
    frame: &[u8],
) -> Result<(), RadioError> {
    let modulation = radio.create_modulation_params(
        SpreadingFactor::_7,
        Bandwidth::_125KHz,
        CodingRate::_4_5,
        FREQUENCY_HZ,
    )?;
    let mut packet = radio.create_tx_packet_params(8, false, true, false, &modulation)?;
    radio
        .prepare_for_tx(&modulation, &mut packet, 10, frame)
        .await?;
    radio.tx().await
}

pub async fn receive<R: RadioKind, D: DelayNs>(
    radio: &mut LoRa<R, D>,
    frame: &mut [u8],
) -> Result<usize, RadioError> {
    let modulation = radio.create_modulation_params(
        SpreadingFactor::_7,
        Bandwidth::_125KHz,
        CodingRate::_4_5,
        FREQUENCY_HZ,
    )?;
    let packet = radio.create_rx_packet_params(8, false, 255, true, false, &modulation)?;
    radio
        .prepare_for_rx(RxMode::Continuous, &modulation, &packet)
        .await?;
    let (len, _) = radio.rx(&packet, frame).await?;
    Ok(len as usize)
}

pub const HELLO: &[u8] = b"RCT-HELLO-1";
pub const CHALLENGE_PREFIX: &[u8] = b"RCT-CHALLENGE-1";
pub const CHALLENGE_SIZE: usize = 16;
