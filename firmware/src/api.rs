//! REST API served over HTTP on port 80.
//!
//! | Method | Path       | Body                | Result                           |
//! |--------|------------|---------------------|----------------------------------|
//! | GET    | `/status`  | -                   | JSON device status               |
//! | POST   | `/display` | `{"text": "hello"}` | Shows the text on the display    |
//! | DELETE | `/display` | -                   | Clears the text                  |
//! | POST   | `/keyboard/type` | `{"text": "...", "delay_ms": 15}` | Types the text on the USB host |
//! | GET    | `/logs`    | -                   | Recent log lines as plain text   |
//! | POST   | `/bootloader` | -                | Reboots into the ROM download mode |
//!
//! Every route needs the header `Authorization: Bearer <API_TOKEN>`.

use core::fmt::Write;
use core::sync::atomic::Ordering;

use embassy_net::Stack;
use embassy_time::Instant;
use heapless::String;
use picoserve::extract::{FromRequestParts, JsonWithUnescapeBufferSize};
use picoserve::request::RequestParts;
use picoserve::response::{IntoResponse, Json, StatusCode};
use picoserve::routing::{PathRouter, get, post};
use picoserve::{Config, Router};
use serde::{Deserialize, Serialize};

use crate::shared::{
    BOOTLOADER, DISPLAY_CMD, DisplayCmd, IP_ADDR, MESSAGE_LEN, RADIO, REQUESTS, RSSI_DBM, TYPE_MAX,
    TYPE_QUEUE, TypeJob, USB_CONFIGURED,
};

// Set at build time, e.g. `API_TOKEN=$(openssl rand -hex 16) cargo run`.
const API_TOKEN: &str = env!("API_TOKEN");

const PORT: u16 = 80;

/// Scratch space for unescaping JSON strings (`\n`, `\"`, ...). It has to hold the longest
/// text accepted by any route, or such requests fail with "Invalid JSON".
const UNESCAPE_BUFFER: usize = TYPE_MAX * 2;

/// Compare without exiting early so timing does not reveal how much of the token matched.
fn token_matches(candidate: &[u8]) -> bool {
    let expected = API_TOKEN.as_bytes();
    let mut diff = (candidate.len() ^ expected.len()) as u8;
    for (i, byte) in expected.iter().enumerate() {
        diff |= byte ^ candidate.get(i).copied().unwrap_or(0);
    }
    diff == 0
}

/// Extractor that rejects the request with 401 unless the bearer token is valid.
struct Authorized;

impl<'r, State> FromRequestParts<'r, State> for Authorized {
    type Rejection = StatusCode;

    async fn from_request_parts(
        _state: &'r State,
        request_parts: &RequestParts<'r>,
    ) -> Result<Self, Self::Rejection> {
        let token = request_parts
            .headers()
            .get("Authorization")
            .and_then(|value| value.as_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));

        match token {
            Some(token) if token_matches(token.as_bytes()) => {
                REQUESTS.fetch_add(1, Ordering::Relaxed);
                Ok(Authorized)
            }
            _ => Err(StatusCode::UNAUTHORIZED),
        }
    }
}

#[derive(Serialize)]
struct WifiBody {
    ip: String<16>,
    rssi_dbm: i32,
}

#[derive(Serialize)]
struct RadioBody {
    ok: bool,
    checks: u32,
    status_byte: u8,
    sync_word: u16,
}

#[derive(Serialize)]
struct UsbBody {
    configured: bool,
}

#[derive(Serialize)]
struct StatusBody {
    uptime_s: u64,
    heap_free: usize,
    requests: u32,
    wifi: WifiBody,
    radio: RadioBody,
    usb: UsbBody,
}

async fn status(_: Authorized) -> impl IntoResponse {
    let mut ip: String<16> = String::new();
    if let Some([a, b, c, d]) = IP_ADDR.lock(|cell| *cell.borrow()) {
        let _ = write!(ip, "{a}.{b}.{c}.{d}");
    }
    let radio = RADIO.lock(|cell| *cell.borrow());

    Json(StatusBody {
        uptime_s: Instant::now().as_secs(),
        heap_free: esp_alloc::HEAP.free(),
        requests: REQUESTS.load(Ordering::Relaxed),
        wifi: WifiBody {
            ip,
            rssi_dbm: RSSI_DBM.load(Ordering::Relaxed),
        },
        radio: RadioBody {
            ok: radio.ok,
            checks: radio.checks,
            status_byte: radio.status_byte,
            sync_word: radio.sync_word,
        },
        usb: UsbBody {
            configured: USB_CONFIGURED.load(Ordering::Relaxed),
        },
    })
}

#[derive(Deserialize)]
struct DisplayRequest {
    text: String<MESSAGE_LEN>,
}

async fn show_display(
    _: Authorized,
    JsonWithUnescapeBufferSize(request): JsonWithUnescapeBufferSize<
        DisplayRequest,
        UNESCAPE_BUFFER,
    >,
) -> StatusCode {
    DISPLAY_CMD.signal(DisplayCmd::Show(request.text));
    StatusCode::NO_CONTENT
}

async fn clear_display(_: Authorized) -> StatusCode {
    DISPLAY_CMD.signal(DisplayCmd::Clear);
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
struct TypeRequest {
    text: String<TYPE_MAX>,
    /// Pause after each key press and release, in milliseconds. Defaults to 15.
    delay_ms: Option<u16>,
}

async fn type_text(
    _: Authorized,
    JsonWithUnescapeBufferSize(request): JsonWithUnescapeBufferSize<TypeRequest, UNESCAPE_BUFFER>,
) -> (StatusCode, &'static str) {
    if !USB_CONFIGURED.load(Ordering::Relaxed) {
        return (StatusCode::new(409), "no USB host is connected\n");
    }
    let job = TypeJob {
        text: request.text,
        delay_ms: request.delay_ms.unwrap_or(15).clamp(1, 200),
    };
    match TYPE_QUEUE.try_send(job) {
        Ok(()) => (StatusCode::new(202), "typing\n"),
        Err(_) => (StatusCode::new(503), "still typing the previous text\n"),
    }
}

async fn logs(_: Authorized) -> impl IntoResponse {
    crate::logger::contents()
}

async fn bootloader(_: Authorized) -> StatusCode {
    BOOTLOADER.signal(());
    StatusCode::NO_CONTENT
}

fn app() -> Router<impl PathRouter> {
    Router::new()
        .route("/status", get(status))
        .route("/display", post(show_display).delete(clear_display))
        .route("/keyboard/type", post(type_text))
        .route("/logs", get(logs))
        .route("/bootloader", post(bootloader))
}

/// Number of HTTP connections served at the same time.
pub const WEB_TASKS: usize = 2;

#[embassy_executor::task(pool_size = WEB_TASKS)]
pub async fn web_task(id: usize, stack: Stack<'static>) -> ! {
    let app = app();
    let config = Config::const_default();

    let mut tcp_rx = [0u8; 1024];
    let mut tcp_tx = [0u8; 1024];
    let mut http_buf = [0u8; 2048];

    picoserve::Server::new(&app, &config, &mut http_buf)
        .listen_and_serve(id, stack, PORT, &mut tcp_rx, &mut tcp_tx)
        .await
        .into_never()
}
