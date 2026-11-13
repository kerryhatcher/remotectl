//! Framing for the host-to-Heltec serial command channel.

use crate::{CommandError, InputCommand, RADIO_TEXT_MAX};

pub const MAGIC: [u8; 4] = *b"RCT1";
pub const HEADER_LEN: usize = 6;
pub const MAX_PAYLOAD: usize = RADIO_TEXT_MAX + 3;
pub const MAX_FRAME: usize = HEADER_LEN + MAX_PAYLOAD;

/// Encode a command as a versioned, length-prefixed serial frame.
pub fn encode(command: &InputCommand<'_>, out: &mut [u8]) -> Result<usize, CommandError> {
    if out.len() < HEADER_LEN {
        return Err(CommandError::BufferTooSmall);
    }
    let len = command.encode(&mut out[HEADER_LEN..])?;
    out[..4].copy_from_slice(&MAGIC);
    out[4..6].copy_from_slice(&(len as u16).to_be_bytes());
    Ok(HEADER_LEN + len)
}

/// Parse an exact frame, rejecting oversized, truncated, and trailing bytes.
pub fn decode(frame: &[u8]) -> Result<InputCommand<'_>, CommandError> {
    if frame.len() < HEADER_LEN || frame[..4] != MAGIC {
        return Err(CommandError::Invalid);
    }
    let len = u16::from_be_bytes([frame[4], frame[5]]) as usize;
    if len > MAX_PAYLOAD || frame.len() != HEADER_LEN + len {
        return Err(CommandError::Invalid);
    }
    InputCommand::decode(&frame[HEADER_LEN..])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames_round_trip_and_reject_bad_lengths() {
        let command = InputCommand::Scroll { amount: -4 };
        let mut frame = [0; MAX_FRAME];
        let n = encode(&command, &mut frame).unwrap();
        assert_eq!(decode(&frame[..n]), Ok(command));
        assert_eq!(decode(&frame[..n - 1]), Err(CommandError::Invalid));
        frame[5] = 255;
        assert_eq!(decode(&frame[..n]), Err(CommandError::Invalid));
    }
}

/// Incremental bounded frame decoder for a byte stream.
pub struct Decoder {
    buffer: [u8; MAX_FRAME],
    len: usize,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub const fn new() -> Self {
        Self {
            buffer: [0; MAX_FRAME],
            len: 0,
        }
    }

    /// Feed one byte. A complete frame remains available until the next byte is fed.
    pub fn push(&mut self, byte: u8) -> Result<Option<&[u8]>, CommandError> {
        if self.len >= HEADER_LEN {
            let length = u16::from_be_bytes([self.buffer[4], self.buffer[5]]) as usize;
            if self.len == HEADER_LEN + length {
                self.len = 0;
            }
        }
        if self.len < MAGIC.len() {
            if byte == MAGIC[self.len] {
                self.buffer[self.len] = byte;
                self.len += 1;
            } else {
                self.len = usize::from(byte == MAGIC[0]);
                if self.len != 0 {
                    self.buffer[0] = byte;
                }
            }
            return Ok(None);
        }
        self.buffer[self.len] = byte;
        self.len += 1;
        if self.len >= HEADER_LEN {
            let length = u16::from_be_bytes([self.buffer[4], self.buffer[5]]) as usize;
            if length > MAX_PAYLOAD || length == 0 {
                self.len = 0;
                return Err(CommandError::Invalid);
            }
            if self.len == HEADER_LEN + length {
                let frame = &self.buffer[..self.len];
                decode(frame)?;
                return Ok(Some(frame));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod stream_tests {
    use super::*;

    #[test]
    fn fragmented_frames_and_resynchronization() {
        let cmd = InputCommand::Move { dx: -9, dy: 4 };
        let mut frame = [0; MAX_FRAME];
        let len = encode(&cmd, &mut frame).unwrap();
        let mut decoder = Decoder::new();
        for byte in b"garbageR" {
            assert_eq!(decoder.push(*byte), Ok(None));
        }
        // An extra leading R must resynchronize to the next magic prefix.
        for byte in &frame[..len - 1] {
            assert_eq!(decoder.push(*byte), Ok(None));
        }
        let result = decoder.push(frame[len - 1]).unwrap().unwrap();
        assert_eq!(decode(result), Ok(cmd));
        for byte in &frame[..len - 1] {
            assert_eq!(decoder.push(*byte), Ok(None));
        }
        assert_eq!(
            decode(decoder.push(frame[len - 1]).unwrap().unwrap()),
            Ok(InputCommand::Move { dx: -9, dy: 4 })
        );
    }

    #[test]
    fn oversized_frame_does_not_poison_next_frame() {
        let mut decoder = Decoder::new();
        for byte in b"RCT1\xff" {
            assert_eq!(decoder.push(*byte), Ok(None));
        }
        assert_eq!(decoder.push(0xff), Err(CommandError::Invalid));
        let mut frame = [0; MAX_FRAME];
        let len = encode(&InputCommand::Scroll { amount: 1 }, &mut frame).unwrap();
        for byte in &frame[..len - 1] {
            assert_eq!(decoder.push(*byte), Ok(None));
        }
        assert!(decoder.push(frame[len - 1]).unwrap().is_some());
    }
}
