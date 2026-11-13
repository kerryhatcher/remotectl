use std::io::{self, Read, Write};
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
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
    /// Type ASCII text on the remote computer.
    Type {
        text: String,
        #[arg(long, default_value_t = 15)]
        delay_ms: u16,
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
enum Button {
    Left,
    Right,
    Middle,
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let input = match &args.command {
        Command::Ping => InputCommand::Ping,
        Command::Type { text, delay_ms } => InputCommand::Type {
            text,
            delay_ms: *delay_ms,
        },
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
    let mut frame = [0; serial::MAX_FRAME];
    let size = serial::encode(&input, &mut frame).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid command: {e:?}"),
        )
    })?;
    let mut port = serialport::new(args.port, 115_200)
        .timeout(Duration::from_secs(10))
        .open()?;
    port.write_all(&frame[..size])?;
    port.flush()?;
    let mut response = [0u8; 1];
    port.read_exact(&mut response)?;
    match response[0] {
        0 => {
            if matches!(args.command, Command::Ping) {
                println!("Receiver authenticated the LoRa ping.");
            } else {
                println!("Receiver accepted the command; USB execution is not confirmed.");
            }
            Ok(())
        }
        1 => Err("sender rejected an invalid serial frame".into()),
        2 => Err("no authenticated radio acknowledgment; execution is unknown".into()),
        3 => Err("sender radio transmission failed".into()),
        4 => Err("sender link keys are not configured".into()),
        5 => Err("receiver USB HID host is not connected".into()),
        6 => Err("receiver input queue is full".into()),
        code => Err(format!("sender rejected command (code {code})").into()),
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("remotectl: {error}");
        std::process::exit(1);
    }
}
