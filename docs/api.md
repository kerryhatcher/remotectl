# REST API

The firmware joins your Wi-Fi network as a station, gets an address by DHCP and serves
HTTP on **port 80**. The address is printed on the serial console and returned by
`/status`:

```
INFO - Network up: http://192.168.1.50/
```

Traffic is plain HTTP, so the bearer token is only reasonably safe on a trusted LAN.
Do not expose the device to the internet.

## Build-time configuration

Credentials and the token are compiled in from environment variables and are never
committed. Set them in the shell that runs `cargo`:

```sh
export WIFI_SSID='my network'
export WIFI_PASSWORD='my password'
export API_TOKEN="$(openssl rand -hex 16)"   # note it down, you need it for every request
cd firmware && cargo run
```

The build fails if any of the three is missing. Changing one triggers a rebuild.
Only WPA2-Personal networks on 2.4 GHz are supported (the ESP32-S3 has no 5 GHz radio).

## Authentication

Every route needs the header `Authorization: Bearer <API_TOKEN>`. A missing or wrong
token gets `401 Unauthorized`.

## Endpoints

### `GET /status`

```sh
curl -H "Authorization: Bearer $API_TOKEN" http://<device-ip>/status
```

```json
{
  "uptime_s": 120,
  "heap_free": 41234,
  "requests": 3,
  "wifi": { "ip": "192.168.1.50", "rssi_dbm": -52 },
  "radio": { "ok": true, "checks": 60, "status_byte": 170, "sync_word": 5156 }
}
```

`radio.sync_word` is `0x1424` (5156) when the SX1262 answers over SPI.
`wifi.rssi_dbm` is refreshed every 5 seconds and is `0` until the first reading.

### `POST /display`

Shows up to 60 characters (3 lines of 20) in the message area of the display.
Non-ASCII characters are replaced with `?`.

```sh
curl -X POST -H "Authorization: Bearer $API_TOKEN" \
     -H "Content-Type: application/json" \
     -d '{"text": "Hello from curl"}' http://<device-ip>/display
```

Returns `204 No Content`. A text longer than 60 characters is rejected.

### `DELETE /display`

Clears the message. Returns `204 No Content`.

## Implementation

| File | Purpose |
|------|---------|
| `firmware/src/wifi.rs` | `esp-radio` station connection, DHCP, `embassy-net` tasks |
| `firmware/src/api.rs` | `picoserve` router, bearer-token extractor, handlers |
| `firmware/src/shared.rs` | State shared with the main loop (radio status, display commands, IP) |
| `firmware/src/bin/main.rs` | Owns the radio and display. Handlers send commands to it through `DISPLAY_CMD` |

Two HTTP tasks run at once (`WEB_TASKS`), so two requests can be served in parallel.
The radio and display stay owned by `main`, because they share one SPI bus.
