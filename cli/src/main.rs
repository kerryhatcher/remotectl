use std::io;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand, ValueEnum};
use rand::Rng;
use remotectl_shared::{InputCommand, MouseButton, serial};

#[derive(Parser)]
#[command(version, about = "Send input commands through a Heltec LoRa sender")]
struct Args {
    /// Serial port of the Heltec sender.
    #[arg(long, env = "REMOTECTL_PORT")]
    port: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check the LoRa handshake without sending HID input.
    Ping,
    /// Continuously move the pointer slightly at random intervals.
    Jiggle,
    /// Type ASCII text on the remote computer.
    Type {
        text: String,
        #[arg(long, default_value_t = 15)]
        delay_ms: u16,
        /// Press macOS Command+Return after typing.
        #[arg(long)]
        command_return: bool,
    },
    /// Enter Unicode using the selected host's Unicode input method.
    Unicode {
        #[arg(value_enum)]
        host: UnicodeHost,
        text: String,
    },
    /// Press up to six USB keyboard usage IDs with an optional modifier bitmask.
    Key {
        /// HID modifier bitmask (e.g. 8 = left Command/Super, 1 = left Control).
        #[arg(long, default_value_t = 0)]
        modifiers: u8,
        /// Key names (enter, escape, arrows, f1–f24, kp0–kp9) or HID usage IDs.
        #[arg(num_args = 0..=6, value_parser = parse_key_usage)]
        usages: Vec<u8>,
    },
    /// Send a media key name or USB consumer-page usage.
    Media {
        #[arg(value_parser = parse_media_usage)]
        usage: u16,
    },
    /// Move the pointer by relative HID counts.
    Move {
        #[arg(allow_hyphen_values = true)]
        dx: i16,
        #[arg(allow_hyphen_values = true)]
        dy: i16,
    },
    /// Click a mouse button.
    Click {
        #[arg(value_enum)]
        button: Button,
        #[arg(long)]
        double: bool,
    },
    /// Scroll the mouse wheel by signed steps.
    Scroll {
        #[arg(allow_hyphen_values = true)]
        amount: i8,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum UnicodeHost {
    Linux,
    Mac,
}

#[derive(Clone, Copy, ValueEnum)]
enum Button {
    Left,
    Right,
    Middle,
}

fn parse_key_usage(input: &str) -> Result<u8, String> {
    let key = input.to_ascii_lowercase();
    let bytes = key.as_bytes();
    if bytes.len() == 1 {
        return match bytes[0] {
            b'a'..=b'z' => Ok(4 + bytes[0] - b'a'),
            b'1'..=b'9' => Ok(30 + bytes[0] - b'1'),
            b'0' => Ok(39),
            _ => Err(format!("unknown key: {input}")),
        };
    }
    if let Some(n) = key.strip_prefix('f').and_then(|v| v.parse::<u8>().ok()) {
        if (1..=12).contains(&n) {
            return Ok(57 + n);
        }
        if (13..=24).contains(&n) {
            return Ok(104 + n - 13);
        }
    }
    if let Some(n) = key.strip_prefix("kp").and_then(|v| v.parse::<u8>().ok()) {
        return match n {
            0 => Ok(98),
            1..=9 => Ok(88 + n),
            _ => Err(format!("unknown key: {input}")),
        };
    }
    match key.as_str() {
        "enter" | "return" => Ok(40),
        "esc" | "escape" => Ok(41),
        "backspace" => Ok(42),
        "tab" => Ok(43),
        "space" => Ok(44),
        "capslock" => Ok(57),
        "printscreen" => Ok(70),
        "scrolllock" => Ok(71),
        "pause" => Ok(72),
        "insert" => Ok(73),
        "home" => Ok(74),
        "pageup" => Ok(75),
        "delete" => Ok(76),
        "end" => Ok(77),
        "pagedown" => Ok(78),
        "right" => Ok(79),
        "left" => Ok(80),
        "down" => Ok(81),
        "up" => Ok(82),
        "numlock" => Ok(83),
        "kpdivide" => Ok(84),
        "kpmultiply" => Ok(85),
        "kpminus" => Ok(86),
        "kpplus" => Ok(87),
        "kpenter" => Ok(88),
        "kpdot" => Ok(99),
        _ => {
            let value = if let Some(hex) = key.strip_prefix("0x") {
                u8::from_str_radix(hex, 16)
            } else {
                key.parse()
            };
            value.map_err(|_| format!("unknown key: {input}"))
        }
    }
}

fn parse_media_usage(input: &str) -> Result<u16, String> {
    match input.to_ascii_lowercase().as_str() {
        "volume-up" => Ok(0xe9),
        "volume-down" => Ok(0xea),
        "mute" => Ok(0xe2),
        "play-pause" => Ok(0xcd),
        "next" => Ok(0xb5),
        "previous" => Ok(0xb6),
        _ => {
            let value = if let Some(hex) = input.strip_prefix("0x") {
                u16::from_str_radix(hex, 16)
            } else {
                input.parse()
            };
            value.map_err(|_| format!("unknown media key: {input}"))
        }
    }
}

/// Returns a random pause and a nonzero, small relative movement.
fn random_jiggle(rng: &mut impl Rng) -> (Duration, i16, i16) {
    let delay = Duration::from_millis(rng.random_range(1_000..=10_000));
    loop {
        let dx = rng.random_range(-12..=12);
        let dy = rng.random_range(-12..=12);
        if dx != 0 || dy != 0 {
            return (delay, dx, dy);
        }
    }
}

fn send(
    port: &mut dyn serialport::SerialPort,
    input: &InputCommand<'_>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut frame = [0; serial::MAX_FRAME];
    let size = serial::encode(input, &mut frame).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid command: {e:?}"),
        )
    })?;
    port.write_all(&frame[..size])?;
    port.flush()?;
    let mut response = [0u8; 1];
    port.read_exact(&mut response)?;
    match response[0] {
        0 => Ok(()),
        1 => Err("sender rejected an invalid serial frame".into()),
        2 => Err("no authenticated radio acknowledgment; execution is unknown".into()),
        3 => Err("sender radio transmission failed".into()),
        4 => Err("sender link keys are not configured".into()),
        5 => Err("receiver USB HID host is not connected".into()),
        6 => Err("receiver input queue is full".into()),
        7 => Err("receiver media-key interface is unavailable".into()),
        8 => Err("receiver USB host is suspended; input was not queued".into()),
        code => Err(format!("sender rejected command (code {code})").into()),
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if let Command::Unicode { host, text } = &args.command {
        if text
            .chars()
            .any(|c| c.is_control() || (matches!(host, UnicodeHost::Mac) && (c as u32) > 0xffff))
        {
            return Err(
                "Unicode input rejects control characters and macOS characters above U+FFFF".into(),
            );
        }
    }
    let mut port = serialport::new(args.port, 115_200)
        .timeout(Duration::from_secs(10))
        .open()?;

    if matches!(args.command, Command::Jiggle) {
        println!("Jiggler running. Press Ctrl+C to stop.");
        let mut rng = rand::rng();
        let mut previous = Instant::now();
        loop {
            let (delay, dx, dy) = random_jiggle(&mut rng);
            std::thread::sleep(delay.saturating_sub(previous.elapsed()));
            previous = Instant::now();
            send(port.as_mut(), &InputCommand::Move { dx, dy })?;
        }
    }

    if let Command::Unicode { host, text } = &args.command {
        let host = match host {
            UnicodeHost::Linux => 1,
            UnicodeHost::Mac => 2,
        };
        for c in text.chars() {
            send(
                port.as_mut(),
                &InputCommand::Unicode {
                    host,
                    scalar: c as u32,
                },
            )?;
        }
        println!("Receiver accepted the Unicode input; host rendering is not confirmed.");
        return Ok(());
    }

    let input = match &args.command {
        Command::Ping => InputCommand::Ping,
        Command::Jiggle | Command::Unicode { .. } => unreachable!(),
        Command::Type {
            text,
            delay_ms,
            command_return,
        } => {
            if *command_return {
                InputCommand::TypeCommandReturn {
                    text,
                    delay_ms: *delay_ms,
                }
            } else {
                InputCommand::Type {
                    text,
                    delay_ms: *delay_ms,
                }
            }
        }
        Command::Key { modifiers, usages } => {
            let mut keys = [0; 6];
            keys[..usages.len()].copy_from_slice(usages);
            InputCommand::KeyChord {
                modifiers: *modifiers,
                keys,
            }
        }
        Command::Media { usage } => InputCommand::Media { usage: *usage },
        Command::Move { dx, dy } => InputCommand::Move { dx: *dx, dy: *dy },
        Command::Click { button, double } => InputCommand::Click {
            button: match button {
                Button::Left => MouseButton::Left,
                Button::Right => MouseButton::Right,
                Button::Middle => MouseButton::Middle,
            },
            double: *double,
        },
        Command::Scroll { amount } => InputCommand::Scroll { amount: *amount },
    };
    send(port.as_mut(), &input)?;
    if matches!(args.command, Command::Ping) {
        println!("Receiver authenticated the LoRa ping.");
    } else {
        println!("Receiver accepted the command; USB execution is not confirmed.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jiggle_has_bounded_delays_and_nonzero_small_movements() {
        let mut rng = rand::rng();
        for _ in 0..1000 {
            let (delay, dx, dy) = random_jiggle(&mut rng);
            assert!((Duration::from_secs(1)..=Duration::from_secs(10)).contains(&delay));
            assert!((-12..=12).contains(&dx) && (-12..=12).contains(&dy));
            assert!(dx != 0 || dy != 0);
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("remotectl: {error}");
        std::process::exit(1);
    }
}
