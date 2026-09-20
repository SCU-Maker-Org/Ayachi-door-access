// Made by Han_feng

#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

pub mod ayachi_core;

use ayachi_core::{MAX_CONNECTIONS, WIFI_RETRY_DELAY};
use ayachi_core::door::door_task;
use ayachi_core::server::{AyachiServer, server_task};
use ayachi_core::SpawnerExt;
use ayachi_core::door::{Door, DoorSignal};
use ayachi_core::mdns::{MdnsAnswers, mdns_task};
use ayachi_core::network::{Network, network_task};
use embassy_futures::select::{Either3, select3};
use embassy_time::Timer;
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::gpio::{Input, InputConfig, Pull};
use esp_hal::timer::timg::TimerGroup;
use esp_println as _;
use esp_radio::wifi::WifiError;
use crate::ayachi_core::{WIFI_PASSWORD, WIFI_SSID};

esp_bootloader_esp_idf::esp_app_desc!();

const HEAP_SIZE: usize = 80; // KB

#[esp_rtos::main]
async fn main(spawner: embassy_executor::Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    esp_alloc::heap_allocator!(size: HEAP_SIZE * 1024);

    let timer_group = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timer_group.timer0, peripherals.FROM_CPU_INTR0);

    let door = Door::init(peripherals.GPIO4);
    let door_signal = DoorSignal::init();

    if let Err(error) = spawner.spawn_task(door_task(door, door_signal)) {
        defmt::error!("Failed to initialize door: {}", error);
        return;
    }

    let button = Input::new(peripherals.GPIO5, InputConfig::default().with_pull(Pull::Up));

    if let Err(error) = spawner.spawn_task(button_task(button, door_signal)) {
        defmt::error!("Failed to spawn button task: {}", error);
        return;
    }

    let (network, runner) = match Network::init(peripherals.WIFI) {
        Ok(pack) => pack,
        Err(error) => {
            defmt::error!("Failed to initialize network: {}", error);
            return;
        }
    };

    if let Err(error) = spawner.spawn_task(network_task(runner)) {
        defmt::error!("Failed to spawn network task: {}", error);
        return;
    }

    let mdns = MdnsAnswers::init();

    if let Err(error) = spawner.spawn_task(mdns_task(mdns, network.stack)) {
        defmt::error!("Failed to spawn mdns task: {}", error);
        return;
    }

    for _ in 0..MAX_CONNECTIONS {
        if let Err(error) = spawner.spawn_task(server_task(AyachiServer{ door, door_signal }, network.stack)) {
            defmt::error!("Failed to spawn Ayachi server task: {}", error);
            return;
        }
    }

    if let Err(error) = network.connect_to(WIFI_SSID, WIFI_PASSWORD) {
        defmt::error!("Failed to set WiFi config: {}", error);
        return;
    }

    'main: loop {
        while let Err(error) = network.connect_async().await {
            defmt::warn!("Failed to connect to wifi(message: {:?}), will retry in {} ms", error, WIFI_RETRY_DELAY);
            Timer::after_millis(WIFI_RETRY_DELAY).await;
        }

        loop {
            match select3(
                network.stack.wait_config_up(),
                network.wait_for_disconnect_async(),
                Timer::after_secs(10),
            ).await {
                Either3::First(_) => break,
                Either3::Second(_) => {
                    defmt::warn!("disconnected before DHCP, reconnecting wifi...");
                    continue 'main;
                },
                Either3::Third(_) => {
                    defmt::warn!("DHCP timeout, retrying...");
                }
            }
        }

        if let Some(ip_config) = network.stack.config_v4() {
            let new_ip = ip_config.address.address();
            defmt::info!("Connected to wifi, IP: {}", new_ip);
            mdns.update_ipv4(new_ip);
        }

        match network.wait_for_disconnect_async().await {
            Ok(info) => {
                defmt::info!("Disconnected from wifi(reason: {:?})", info.reason);
            },
            Err(WifiError::NotConnected) => {
                defmt::info!("Disconnected from wifi before waiting for disconnect");
            }
            Err(error) => {
                defmt::error!("An error occurred during wait for wifi disconnect: {:?}", error);
                return;
            }
        }
    }
}

#[embassy_executor::task]
async fn button_task(mut button: Input<'static>, signal: &'static DoorSignal) {
    loop {
        button.wait_for_falling_edge().await;

        signal.open_once.signal(());

        button.wait_for_high().await;
    }
}