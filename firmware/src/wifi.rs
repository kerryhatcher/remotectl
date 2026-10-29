//! Wi-Fi station connection, DHCP and the network stack tasks.

use core::sync::atomic::Ordering;

use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_net::{Runner, Stack, StackResources};
use embassy_time::{Duration, Timer};
use esp_radio::wifi::sta::StationConfig;
use esp_radio::wifi::{AuthenticationMethodConfig, Config, Interface, WifiController};
use log::{info, warn};
use static_cell::StaticCell;

use crate::shared::{IP_ADDR, RSSI_DBM};

// Set at build time, e.g. `WIFI_SSID=... WIFI_PASSWORD=... API_TOKEN=... cargo run`.
const SSID: &str = env!("WIFI_SSID");
const PASSWORD: &str = env!("WIFI_PASSWORD");

/// Sockets for the HTTP server tasks plus DHCP and slack.
const STACK_SOCKETS: usize = 6;

static RESOURCES: StaticCell<StackResources<STACK_SOCKETS>> = StaticCell::new();

/// Bring up Wi-Fi in station mode and return the network stack.
///
/// The stack is usable immediately; it gets an address once the connection task has
/// joined the access point and DHCP has finished (see [`crate::shared::IP_ADDR`]).
pub fn start(
    spawner: &Spawner,
    wifi: esp_hal::peripherals::WIFI<'static>,
    seed: u64,
) -> Stack<'static> {
    let controller =
        WifiController::new(wifi, Default::default()).expect("Failed to initialize Wi-Fi");
    let interface = Interface::station();

    let (stack, runner) = embassy_net::new(
        interface,
        embassy_net::Config::dhcpv4(Default::default()),
        RESOURCES.init(StackResources::new()),
        seed,
    );

    spawner.spawn(connection_task(controller).expect("connection task"));
    spawner.spawn(net_task(runner).expect("net task"));
    spawner.spawn(ip_task(stack).expect("ip task"));

    stack
}

#[embassy_executor::task]
async fn connection_task(mut controller: WifiController<'static>) {
    let config = Config::Station(
        StationConfig::default()
            .with_ssid(SSID.try_into().expect("WIFI_SSID too long"))
            .with_authentication(AuthenticationMethodConfig::Wpa2Personal(
                PASSWORD.try_into().expect("WIFI_PASSWORD invalid"),
            )),
    );
    controller.set_config(&config).expect("wifi config");

    loop {
        info!("Wi-Fi: connecting to '{SSID}'");
        match controller.connect_async().await {
            Ok(_) => {
                info!("Wi-Fi: connected");
                // Refresh the RSSI until the link drops.
                loop {
                    match select(
                        controller.wait_for_disconnect_async(),
                        Timer::after(Duration::from_secs(5)),
                    )
                    .await
                    {
                        Either::First(_) => break,
                        Either::Second(()) => {
                            if let Ok(rssi) = controller.rssi() {
                                RSSI_DBM.store(rssi, Ordering::Relaxed);
                            }
                        }
                    }
                }
                warn!("Wi-Fi: disconnected");
            }
            Err(e) => warn!("Wi-Fi: connect failed: {e:?}"),
        }
        Timer::after(Duration::from_secs(5)).await;
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await
}

/// Publish the DHCP address in [`IP_ADDR`] while the link is configured.
#[embassy_executor::task]
async fn ip_task(stack: Stack<'static>) {
    loop {
        stack.wait_config_up().await;
        if let Some(config) = stack.config_v4() {
            let ip = config.address.address();
            info!("Network up: http://{ip}/");
            IP_ADDR.lock(|cell| *cell.borrow_mut() = Some(ip.octets()));
        }
        stack.wait_config_down().await;
        warn!("Network down");
        IP_ADDR.lock(|cell| *cell.borrow_mut() = None);
    }
}
