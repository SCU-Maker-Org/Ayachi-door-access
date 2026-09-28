// Made by Han_feng

use esp_bootloader_esp_idf::partitions::{read_partition_table, PARTITION_TABLE_MAX_LEN, PartitionType, DataPartitionSubType};
use esp_hal::peripherals::{FLASH, LPWR};
use esp_storage::FlashStorage;

pub mod server;
pub mod mdns;
pub mod network;
pub mod door;
pub mod system;
pub mod utils;

include!(concat!(env!("OUT_DIR"), "/server_config.rs"));

const FACTORY_REQUEST: u32 = 0x0d000721;

// Timestamp
#[unsafe(no_mangle)]
pub extern "Rust" fn _esp_println_timestamp() -> u64 {
    esp_hal::time::Instant::now().duration_since_epoch().as_millis()
}

// Panic reset
#[unsafe(no_mangle)]
pub extern "Rust" fn custom_halt() -> ! {
    let regs = LPWR::regs();
    let next_count = match regs.store7().read().bits() {
        count @ 0..PANIC_RETRY => count + 1,
        PANIC_RETRY | FACTORY_REQUEST => FACTORY_REQUEST,
        _ => 1,
    };

    regs.store7().write(|reg| unsafe { reg.bits(next_count) });
    esp_hal::system::software_reset()
}

pub fn factory_request_reset() {
    LPWR::regs().store7().write(|reg| unsafe { reg.bits(0) });
}

pub fn factory_request(flash: &FLASH) {
    match LPWR::regs().store7().read().bits() {
        count @ 1..=PANIC_RETRY => match PANIC_RETRY - count {
            0 => defmt::warn!("Factory request count: {}, system will reset to factory next time", count),
            left => defmt::warn!("Factory request count: {}, system will reset to factory in {} times", count, left)
        },
        FACTORY_REQUEST => {
            defmt::warn!("Factory reset request received");

            let mut flash = FlashStorage::new(unsafe { flash.clone_unchecked() }).multicore_auto_park();
            let mut buffer = [0; PARTITION_TABLE_MAX_LEN];

            let table = match read_partition_table(&mut flash, &mut buffer) {
                Ok(table) => table,
                Err(error) => {
                    defmt::warn!("Failed to read partition table: {}, system will keep the current partition", error);
                    return;
                },
            };

            let mut ota_region = match table.find_partition(PartitionType::Data(DataPartitionSubType::Ota)) {
                Ok(Some(entry)) => entry.as_flash_region(&mut flash),
                Ok(None) => {
                    defmt::warn!("ota table not found, system will keep the current partition");
                    return;
                },
                Err(error) => {
                    defmt::warn!("Failed to find ota region: {}, system will keep the current partition", error);
                    return;
                }
            };

            match ota_region.erase(0, ota_region.capacity() as u32) {
                Ok(_) => {
                    defmt::warn!("cleared the ota region, the next boot should go to factory");
                    factory_request_reset();
                    esp_hal::system::software_reset()
                },
                Err(error) => defmt::warn!("Failed to erase Ota region: {}, system will keep the current partition", error)
            }
        },
        _ => (),
    }
}