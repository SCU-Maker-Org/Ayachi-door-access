// Made by Han_feng

pub mod server;
pub mod mdns;
pub mod network;
pub mod door;
pub mod system;
pub mod utils;

include!(concat!(env!("OUT_DIR"), "/server_config.rs"));

#[unsafe(no_mangle)]
pub extern "Rust" fn _esp_println_timestamp() -> u64 {
    esp_hal::time::Instant::now().duration_since_epoch().as_millis()
}