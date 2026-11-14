#![no_std]

/// Maximum number of characters shown in the XIAO display message area.
pub const DISPLAY_MESSAGE_MAX: usize = 60;
/// Maximum number of bytes accepted by a keyboard typing request.
pub const KEYBOARD_TEXT_MAX: usize = 256;

pub mod input_expiry;
#[cfg(feature = "radio")]
pub mod radio;
pub mod secure;
pub mod serial;

/// Maximum text bytes in one radio command. Longer text must be split by the sender.
pub const RADIO_TEXT_MAX: usize = 120;

/// A single remote input action, independent of either board's USB implementation.
#[derive(Debug, PartialEq, Eq)]
pub enum InputCommand<'a> {
    /// Check the radio handshake without sending HID input.
    Ping,
    Type {
        text: &'a str,
        delay_ms: u16,
    },
    TypeCommandReturn {
        text: &'a str,
        delay_ms: u16,
    },
    /// Keyboard-page usage IDs held with the given HID modifier bit mask.
    KeyChord {
        modifiers: u8,
        keys: [u8; 6],
    },
    /// A USB consumer-page usage (media controls).
    Media {
        usage: u16,
    },
    /// A Unicode scalar using a host-specific input method (1=Linux, 2=macOS).
    Unicode {
        host: u8,
        scalar: u32,
    },
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CommandError {
    Invalid,
    BufferTooSmall,
}

impl InputCommand<'_> {
    /// Encode the command into a caller-provided buffer. This does not authenticate it.
    pub fn encode(&self, output: &mut [u8]) -> Result<usize, CommandError> {
        let needed = match self {
            Self::Ping => 1,
            Self::Type { text, delay_ms } | Self::TypeCommandReturn { text, delay_ms } => {
                if text.len() > RADIO_TEXT_MAX || !text.is_ascii() || !(1..=200).contains(delay_ms)
                {
                    return Err(CommandError::Invalid);
                }
                3 + text.len()
            }
            Self::KeyChord { modifiers, keys } => {
                if *modifiers == 0 && keys.iter().all(|key| *key == 0) {
                    return Err(CommandError::Invalid);
                }
                8
            }
            Self::Media { usage } => {
                if *usage == 0 || *usage > 0x03ff {
                    return Err(CommandError::Invalid);
                }
                3
            }
            Self::Unicode { host, scalar } => {
                if !matches!(host, 1 | 2)
                    || char::from_u32(*scalar).is_none()
                    || (*host == 2 && *scalar > 0xffff)
                {
                    return Err(CommandError::Invalid);
                }
                6
            }
            Self::Move { .. } => 5,
            Self::Click { .. } => 3,
            Self::Scroll { amount } => {
                if *amount == i8::MIN {
                    return Err(CommandError::Invalid);
                }
                2
            }
        };
        if output.len() < needed {
            return Err(CommandError::BufferTooSmall);
        }
        match self {
            Self::Ping => output[0] = 5,
            Self::Type { text, delay_ms } => {
                output[0] = 1;
                output[1..3].copy_from_slice(&delay_ms.to_be_bytes());
                output[3..needed].copy_from_slice(text.as_bytes());
            }
            Self::TypeCommandReturn { text, delay_ms } => {
                output[0] = 6;
                output[1..3].copy_from_slice(&delay_ms.to_be_bytes());
                output[3..needed].copy_from_slice(text.as_bytes());
            }
            Self::KeyChord { modifiers, keys } => {
                output[0] = 7;
                output[1] = *modifiers;
                output[2..8].copy_from_slice(keys);
            }
            Self::Media { usage } => {
                output[0] = 8;
                output[1..3].copy_from_slice(&usage.to_be_bytes());
            }
            Self::Unicode { host, scalar } => {
                output[0] = 9;
                output[1] = *host;
                output[2..6].copy_from_slice(&scalar.to_be_bytes());
            }
            Self::Move { dx, dy } => {
                output[0] = 2;
                output[1..3].copy_from_slice(&dx.to_be_bytes());
                output[3..5].copy_from_slice(&dy.to_be_bytes());
            }
            Self::Click { button, double } => {
                output[0] = 3;
                output[1] = match button {
                    MouseButton::Left => 1,
                    MouseButton::Right => 2,
                    MouseButton::Middle => 3,
                };
                output[2] = u8::from(*double);
            }
            Self::Scroll { amount } => {
                output[0] = 4;
                output[1] = *amount as u8;
            }
        }
        Ok(needed)
    }

    /// Decode an authenticated command payload; reject malformed or trailing bytes.
    pub fn decode(input: &[u8]) -> Result<InputCommand<'_>, CommandError> {
        match input {
            [5] => Ok(InputCommand::Ping),
            [opcode @ (1 | 6), hi, lo, text @ ..] if text.len() <= RADIO_TEXT_MAX => {
                let delay_ms = u16::from_be_bytes([*hi, *lo]);
                let text = core::str::from_utf8(text).map_err(|_| CommandError::Invalid)?;
                if !text.is_ascii() || !(1..=200).contains(&delay_ms) {
                    return Err(CommandError::Invalid);
                }
                if *opcode == 1 {
                    Ok(InputCommand::Type { text, delay_ms })
                } else {
                    Ok(InputCommand::TypeCommandReturn { text, delay_ms })
                }
            }
            [7, modifiers, a, b, c, d, e, f]
                if *modifiers != 0 || [*a, *b, *c, *d, *e, *f].iter().any(|key| *key != 0) =>
            {
                Ok(InputCommand::KeyChord {
                    modifiers: *modifiers,
                    keys: [*a, *b, *c, *d, *e, *f],
                })
            }
            [8, hi, lo]
                if u16::from_be_bytes([*hi, *lo]) > 0
                    && u16::from_be_bytes([*hi, *lo]) <= 0x03ff =>
            {
                Ok(InputCommand::Media {
                    usage: u16::from_be_bytes([*hi, *lo]),
                })
            }
            [9, host @ (1 | 2), a, b, c, d] => {
                let scalar = u32::from_be_bytes([*a, *b, *c, *d]);
                if char::from_u32(scalar).is_none() || (*host == 2 && scalar > 0xffff) {
                    return Err(CommandError::Invalid);
                }
                Ok(InputCommand::Unicode {
                    host: *host,
                    scalar,
                })
            }
            [2, xh, xl, yh, yl] => Ok(InputCommand::Move {
                dx: i16::from_be_bytes([*xh, *xl]),
                dy: i16::from_be_bytes([*yh, *yl]),
            }),
            [3, button @ 1..=3, double @ 0..=1] => Ok(InputCommand::Click {
                button: match button {
                    1 => MouseButton::Left,
                    2 => MouseButton::Right,
                    _ => MouseButton::Middle,
                },
                double: *double == 1,
            }),
            [4, amount] if *amount != 0x80 => Ok(InputCommand::Scroll {
                amount: *amount as i8,
            }),
            _ => Err(CommandError::Invalid),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_commands_round_trip() {
        let commands = [
            InputCommand::Type {
                text: "hello\n",
                delay_ms: 15,
            },
            InputCommand::TypeCommandReturn {
                text: "test 2",
                delay_ms: 15,
            },
            InputCommand::KeyChord {
                modifiers: 0x08,
                keys: [0x28, 0, 0, 0, 0, 0],
            },
            InputCommand::Media { usage: 0xe9 },
            InputCommand::Unicode {
                host: 1,
                scalar: 0x1f600,
            },
            InputCommand::Unicode {
                host: 2,
                scalar: 0x00e9,
            },
            InputCommand::Move { dx: -1000, dy: 400 },
            InputCommand::Click {
                button: MouseButton::Middle,
                double: true,
            },
            InputCommand::Scroll { amount: -127 },
        ];
        for command in commands {
            let mut buffer = [0; RADIO_TEXT_MAX + 3];
            let len = command.encode(&mut buffer).unwrap();
            assert_eq!(InputCommand::decode(&buffer[..len]), Ok(command));
        }
    }

    #[test]
    fn rejects_invalid_and_truncated_commands() {
        for input in [
            &[][..],
            &[2, 1, 2][..],
            &[3, 4, 0][..],
            &[3, 1, 2][..],
            &[4, 1, 0][..],
            &[1, 0, 0][..],
            &[1, 0, 1, 0xff][..],
        ] {
            assert_eq!(InputCommand::decode(input), Err(CommandError::Invalid));
        }
        assert_eq!(
            InputCommand::Move { dx: 0, dy: 0 }.encode(&mut [0; 4]),
            Err(CommandError::BufferTooSmall)
        );
        assert_eq!(
            InputCommand::Scroll { amount: i8::MIN }.encode(&mut [0; 2]),
            Err(CommandError::Invalid)
        );
        assert_eq!(InputCommand::decode(&[4, 0x80]), Err(CommandError::Invalid));
        assert_eq!(
            InputCommand::Type {
                text: "é",
                delay_ms: 15
            }
            .encode(&mut [0; 16]),
            Err(CommandError::Invalid)
        );
    }
}

/// Parse a 32-byte key encoded as 64 hexadecimal ASCII characters.
pub fn parse_key(hex: &str) -> Option<[u8; 32]> {
    fn nibble(value: u8) -> Option<u8> {
        match value {
            b'0'..=b'9' => Some(value - b'0'),
            b'a'..=b'f' => Some(value - b'a' + 10),
            b'A'..=b'F' => Some(value - b'A' + 10),
            _ => None,
        }
    }
    let input = hex.as_bytes();
    if input.len() != 64 {
        return None;
    }
    let mut out = [0; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = nibble(input[i * 2])? << 4 | nibble(input[i * 2 + 1])?;
    }
    Some(out)
}
