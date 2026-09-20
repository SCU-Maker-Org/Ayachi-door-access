// Made by Han_feng

use embassy_executor::{SpawnToken, Spawner};

pub mod server;
pub mod mdns;
pub mod network;
pub mod door;

include!(concat!(env!("OUT_DIR"), "/server_config.rs"));

#[unsafe(no_mangle)]
pub extern "Rust" fn _esp_println_timestamp() -> u64 {
    esp_hal::time::Instant::now().duration_since_epoch().as_millis()
}

pub trait SpawnerExt{
    fn spawn_task<S, E>(&self, task: Result<SpawnToken<S>, E>) -> Result<(), E>;
}

impl SpawnerExt for Spawner{
    fn spawn_task<S, E>(&self, task: Result<SpawnToken<S>, E>) -> Result<(), E>
    {
        task.map(|task| self.spawn(task))
    }
}