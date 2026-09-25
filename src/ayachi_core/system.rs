// Made by Han_feng

use crate::stmt_join;
use super::utils::{CompactFormat, TextBytes, ContainerExt};
use core::cell::{Cell, RefCell};
use core::fmt::{Debug, Formatter, Write};
use core::ops::DerefMut;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use embassy_futures::yield_now;
use embassy_sync::blocking_mutex::CriticalSectionMutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::Timer;
use embedded_storage::nor_flash::NorFlash;
use esp_bootloader_esp_idf::EspAppDesc;
use esp_bootloader_esp_idf::ota::OtaImageState;
use esp_bootloader_esp_idf::ota_updater::OtaUpdater;
use esp_bootloader_esp_idf::partitions::{AppPartitionSubType, Error, FlashRegion, NorFlashRegion, PARTITION_TABLE_MAX_LEN, PartitionEntry, PartitionType, read_partition_table};
use esp_hal::__macro_implementation::static_cell::StaticCell;
use esp_hal::peripherals::FLASH;
use esp_storage::FlashStorage;
use pbkdf2::sha2::{Digest, Sha256};
use serde::Deserialize;

// Enums
#[derive(Copy, Clone, Debug, defmt::Format)]
pub enum SystemError {
    Uploading,
    OutOfMemory,
    Incomplete,
    WriteFailure,
    OTA(Error),
}

#[derive(Default, Eq, PartialEq, Copy, Clone, Debug, defmt::Format)]
pub enum UploadState {
    #[default]
    Idle,
    Received,
    Erasing,
    Writing,
    Identifying,
    Ready,
    Failed,
}

#[derive(Default, Eq, PartialEq, Copy, Clone, Debug, defmt::Format)]
pub enum PartitionState {
    #[default]
    Unavailable,
    Empty,
    Unknown,
    Broken,
    Unverified,
    Foreign,
    Available
}

#[derive(Eq, PartialEq, Copy, Clone, defmt::Format, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PartitionSlot{
    Factory,
    Current,
    Next
}

// Structs
pub struct System<'u> {
    uploading: AtomicBool,
    activate_countdown: AtomicU32,
    activate_signal: Signal<CriticalSectionRawMutex, ()>,
    inner: CriticalSectionMutex<SystemRaw<'u>>
}

struct SystemRaw<'u> {
    flash: RefCell<FlashStorage<'u>>,
    upload_process: RefCell<UploadProcess>,
    factory: Option<Partition>,
    current: Option<Partition>,
    next: Option<Partition>,
}

#[derive(Default, Copy, Clone, defmt::Format)]
pub struct UploadProcess {
    pub state: UploadState,
    pub message: Option<SystemError>,
    pub erase: [u32; 2], // [current, total]
    pub write: [u32; 2], // [current, total]
}

pub struct Partition {
    entry: PartitionEntry,
    info: Cell<PartitionInfo>,
}

#[derive(Default, Copy, Clone, defmt::Format)]
pub struct PartitionInfo {
    pub state: PartitionState,

    // Image header
    pub segment_count: Option<u8>,
    pub entry_address: Option<u32>,
    pub chip_id: Option<u16>,
    pub hash_appended: Option<bool>,

    // Application descriptor
    pub secure_version: Option<u32>,
    pub version: Option<TextBytes<32>>,
    pub project: Option<TextBytes<32>>,
    pub time: Option<TextBytes<16>>,
    pub date: Option<TextBytes<16>>,
    pub sha256: Option<[u8; 32]>,

    pub digest_sha256: Option<[u8; 32]>,
}

struct UploadGuard<'g>(&'g AtomicBool);

// Statics
static SYSTEM: StaticCell<System> = StaticCell::new();

// Impls
impl<'u> System<'u> {
    pub async fn init(flash: FLASH<'static>) -> &'static System<'static> {
        let mut flash = FlashStorage::new(flash).multicore_auto_park();
        let mut buffer = [0; PARTITION_TABLE_MAX_LEN];

        let next_type = OtaUpdater::new(&mut flash, &mut buffer).ok().and_then(|mut updater| updater.next_partition().map(|(_, next_type)| next_type).ok());

        let mut result = OtaUpdater::new(&mut flash, &mut buffer).ok().unwrap();
        defmt::info!("next: {:?}", result.next_partition());

        let (factory, current, next) = match read_partition_table(&mut flash, &mut buffer) {
            Ok(table) => (
                    table.find_partition(PartitionType::App(AppPartitionSubType::Factory)).ok().flatten(),
                    table.booted_partition().ok().flatten(),
                    next_type.map(|next_type| table.find_partition(PartitionType::App(next_type)).ok()).flatten().flatten()
            ),
            Err(_) => (None, None, None),
        };

        let system = System {
            uploading: AtomicBool::new(false),
            activate_countdown: AtomicU32::new(u32::MAX),
            activate_signal: Signal::new(),
            inner: CriticalSectionMutex::new(SystemRaw {
                flash: RefCell::new(flash),
                upload_process: RefCell::new(UploadProcess::default()),
                factory: factory.map(|entry| Partition::new(entry)),
                current: current.map(|entry| Partition::new(entry)),
                next: next.map(|entry| Partition::new(entry))
            })
        };

        for (entry, slot) in [
            (factory, PartitionSlot::Factory),
            (current, PartitionSlot::Current),
            (next, PartitionSlot::Next)
        ] {
            entry.async_map(async |entry| system.inspect(entry, None).await).await.map(|info| system.set_info(slot, info));
        }

        SYSTEM.init(system)
    }

    pub async fn upload<F, E>(&self, file_size: usize, file_stream: F) -> Result<(), SystemError>
    where
        F: AsyncFnMut(&mut [u8]) -> Result<usize, E>,
        E: Into<SystemError>
    {
        if file_size == 0 || file_size % FlashStorage::WRITE_SIZE != 0 {
            return Err(Error::NotSupported.into());
        }

        if self.uploading.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            return Err(SystemError::Uploading);
        }

        let _guard = UploadGuard(&self.uploading);

        self.process(|process| {
            process.state = UploadState::Received;
            process.message = None;
        });

        if let Err(error) = self.system_upload(file_size, file_stream).await && let Some(next) = self.inner.lock(|system| {
            let mut process = system.upload_process.borrow_mut();
            process.message = Some(error);
            (process.state != UploadState::Received).then_some(system.next.as_ref().map(|next| next.entry)).flatten()
        }) {
            self.set_info(PartitionSlot::Next, self.inspect(next, None).await);
        }

        Ok(())
    }

    pub fn activate_next(&self) -> Result<(), SystemError> {
        static COUNTDOWN_SECONDS: u32 = 5;

        self.check_activatable()?;

        self.flash(|flash| -> Result<(), Error> {
            let mut buffer = [0; PARTITION_TABLE_MAX_LEN];
            let mut updater = OtaUpdater::new(flash, &mut buffer)?;

            updater.activate_next_partition()?;
            updater.set_current_ota_state(OtaImageState::New)?;

            Ok(())
        })?;

        self.activate_countdown.store(COUNTDOWN_SECONDS, Ordering::Relaxed);
        self.activate_signal.signal(());

        Ok(())
    }

    pub fn is_uploading(&self) -> bool {
        self.uploading.load(Ordering::Relaxed)
    }

    pub fn activate_countdown(&self) -> Option<u32> {
        let countdown = self.activate_countdown.load(Ordering::Relaxed);
        (countdown != u32::MAX).then_some(countdown)
    }

    pub fn get_process(&self) -> UploadProcess {
        self.process(|process| *process)
    }

    pub fn get_partition_info(&self, slot: PartitionSlot) -> Option<PartitionInfo> {
        self.partition(slot, |partition| partition.map(|partition| partition.info.get()))
    }

    pub fn get_partition_infos(&self) -> [Option<PartitionInfo>; 3] {
        self.inner.lock(|system| [
            system.factory.as_ref().map(|factory| factory.info.get()),
            system.current.as_ref().map(|current| current.info.get()),
            system.next.as_ref().map(|next| next.info.get()),
        ])
    }
}

impl<'u> System<'u> {
    async fn system_upload<F, E>(&self, file_size: usize, mut file_stream: F) -> Result<(), SystemError>
    where
        F: AsyncFnMut(&mut [u8]) -> Result<usize, E>,
        E: Into<SystemError>
    {
        let Some(target_entry) = self.inner.lock(|system| system.next.as_ref().map(|next| next.entry)) else {
            return Err(Error::NotSupported.into());
        };

        // Erase the partition
        let (distribution, total) = Self::calculate_erase_distribution(file_size, self.region(target_entry, |target| target.capacity()))?;
        let mut erased = 0;
        self.inner.lock(|system| {
            let mut process = system.upload_process.borrow_mut();
            process.state = UploadState::Erasing;
            process.erase = [erased, total as u32];

            system.next.as_ref().map(|next| next.info.set(PartitionInfo::default()));
        });

        for _ in 0..distribution[0] {
            // block erase
            self.region(target_entry, |mut target| target.erase(erased, erased + FlashStorage::BLOCK_SIZE))?;
            erased += FlashStorage::BLOCK_SIZE;
            self.process(|process| process.erase[0] = erased);
            yield_now().await;
        }

        for _ in 0..distribution[1] {
            // sector erase
            self.region(target_entry, |mut target| target.erase(erased, erased + FlashStorage::ERASE_SIZE as u32))?;
            erased += FlashStorage::ERASE_SIZE as u32;
            self.process(|process| process.erase[0] = erased);
            yield_now().await;
        }

        // Write the partition
        let mut written = 0;
        let mut write_buffer = [0; FlashStorage::SECTOR_SIZE as usize];
        let mut buffer_length = 0;
        self.process(|process| {
            process.state = UploadState::Writing;
            process.write[0] = written;
            process.write[1] = file_size as u32;
        });

        loop {
            match file_stream(&mut write_buffer[buffer_length..]).await {
                Ok(0) => {
                    if (written as usize) + buffer_length < file_size {
                        return Err(SystemError::Incomplete);
                    }

                    if buffer_length > 0 {
                        self.nor_region(target_entry, |mut target| target.write(written, &write_buffer[..buffer_length]))??;
                        self.process(|process| process.write[0] = written + buffer_length as u32);
                    }

                    break;
                },
                Ok(size) => {
                    buffer_length += size;

                    if (written as usize) + buffer_length > file_size {
                        return Err(SystemError::OutOfMemory);
                    }

                    if buffer_length == write_buffer.len() {
                        self.nor_region(target_entry, |mut target| target.write(written, &write_buffer))??;
                        written += buffer_length as u32;
                        self.process(|process| process.write[0] = written);
                        buffer_length = 0;
                    }
                },
                Err(error) => return Err(error.into())
            }
        }

        // identify
        self.process(|process| process.state = UploadState::Identifying);

        let partition_info = self.inspect(target_entry, Some(file_size)).await;

        self.inner.lock(|system| {
            system.upload_process.borrow_mut().state = partition_info.activatable().then_some(UploadState::Ready).unwrap_or(UploadState::Failed);
            system.next.as_ref().map(|next| next.info.set(partition_info));
        });

        Ok(())
    }

    async fn inspect(&self, entry: PartitionEntry, expected_length: Option<usize>) -> PartitionInfo {
        static IMAGE_OFFSET: usize = 0;
        static IMAGE_HEADER: usize = 24;
        static SEGMENT_HEADER: usize = 8;
        static DESCRIPTOR_OFFSET: usize = IMAGE_OFFSET + IMAGE_HEADER + SEGMENT_HEADER;
        static PARTITION_HEADER_LENGTH: usize = DESCRIPTOR_OFFSET + size_of::<EspAppDesc>();

        static IMAGE_MAGIC: u8 = 0xE9;
        static DESCRIPTOR_MAGIC: u32 = 0xABCD5432;
        static MAX_SEGMENTS: u8 = 16;
        static PROJECT_NAME: &[u8] = env!("CARGO_PKG_NAME").as_bytes();

        let mut result = PartitionInfo::default();
        let mut buffer = [0; PARTITION_HEADER_LENGTH];

        if self.region(entry, |mut region| region.read(0, &mut buffer)).is_err() {
            return result;
        };

        // Image header
        if buffer[IMAGE_OFFSET] != IMAGE_MAGIC {
            result.state = PartitionState::Empty;
            return result;
        }
        result.segment_count = Some(buffer[IMAGE_OFFSET + 1]);
        result.entry_address = Some(u32::from_le_bytes(buffer[IMAGE_OFFSET + 4..IMAGE_OFFSET + 8].try_into().unwrap()));
        result.chip_id = Some(u16::from_le_bytes(buffer[IMAGE_OFFSET + 12..IMAGE_OFFSET + 14].try_into().unwrap()));
        result.hash_appended = Some(buffer[IMAGE_OFFSET + 23] != 0);

        // Application descriptor
        if u32::from_le_bytes(buffer[DESCRIPTOR_OFFSET..DESCRIPTOR_OFFSET + 4].try_into().unwrap()) != DESCRIPTOR_MAGIC {
            result.state = PartitionState::Unknown;
            return result;
        }
        result.secure_version = Some(u32::from_le_bytes(buffer[DESCRIPTOR_OFFSET + 4..DESCRIPTOR_OFFSET + 8].try_into().unwrap()));
        result.version = Some(buffer[DESCRIPTOR_OFFSET + 16..DESCRIPTOR_OFFSET + 48].try_into().unwrap());
        result.project = Some(buffer[DESCRIPTOR_OFFSET + 48..DESCRIPTOR_OFFSET + 80].try_into().unwrap());
        result.time = Some(buffer[DESCRIPTOR_OFFSET + 80..DESCRIPTOR_OFFSET + 96].try_into().unwrap());
        result.date = Some(buffer[DESCRIPTOR_OFFSET + 96..DESCRIPTOR_OFFSET + 112].try_into().unwrap());
        result.sha256 = Some(buffer[DESCRIPTOR_OFFSET + 144..DESCRIPTOR_OFFSET + 176].try_into().unwrap());

        // integrity check
        let mut cursor = IMAGE_HEADER as u32;
        let mut segment_buffer = [0; SEGMENT_HEADER];
        let partition_length = entry.len();

        let Some(segments) = result.segment_count.filter(|count| MAX_SEGMENTS.ge(count)) else {
            result.state = PartitionState::Broken;
            return result;
        };

        for _ in 0..segments {
            if self.region(entry, |mut region| region.read(cursor, &mut segment_buffer)).is_err() {
                result.state = PartitionState::Broken;
                return result;
            };

            let segment_length = u32::from_le_bytes(segment_buffer[4..8].try_into().unwrap());
            if segment_length & 3 != 0 || segment_length > partition_length {
                result.state = PartitionState::Broken;
                return result;
            }

            cursor += (SEGMENT_HEADER as u32) + segment_length;
        }

        let hash_appended = result.hash_appended.unwrap_or(false);
        let image_length = (cursor + 1).next_multiple_of(16); // checksum: 1 byte
        if expected_length.is_some_and(|expected| image_length + hash_appended.then_some(32).unwrap_or(0) != expected as u32) {
            result.state = PartitionState::Broken;
            return result;
        }

        if image_length < PARTITION_HEADER_LENGTH as u32 || image_length > partition_length {
            result.state = PartitionState::Broken;
            return result;
        }

        let Ok(sha256) = self.digest(entry, image_length as usize).await else {
            result.state = PartitionState::Broken;
            return result;
        };

        result.digest_sha256 = Some(sha256);

        if !hash_appended {
            result.state = PartitionState::Unverified;
            return result;
        }

        let mut expected_sha256 = [0; 32];
        if self.region(entry, |mut region| region.read(image_length, &mut expected_sha256)).is_err() {
            result.state = PartitionState::Broken;
            return result;
        }

        if sha256.ne(&expected_sha256) {
            result.state = PartitionState::Broken;
            return result;
        }

        // foreign check
        if result.project.is_some_and(|project| !project.starts_with(PROJECT_NAME)) {
            result.state = PartitionState::Foreign;
            return result;
        }

        result.state = PartitionState::Available;
        result
    }

    async fn digest(&self, entry: PartitionEntry, length: usize) -> Result<[u8; 32], SystemError> {
        let mut hasher = Sha256::new();
        let mut buffer = [0; FlashStorage::SECTOR_SIZE as usize];
        let mut offset = 0;

        while offset < length {
            let read_length = buffer.len().min(length - offset);
            self.region(entry, |mut region| region.read(offset as u32, &mut buffer[..read_length]))?;
            hasher.update(&buffer[..read_length]);
            offset += read_length;
            yield_now().await;
        }

        Ok(hasher.finalize().into())
    }

    fn check_activatable(&self) -> Result<(), SystemError> {
        let Some(info) = self.get_partition_info(PartitionSlot::Next) else {
            return Err(Error::NotSupported.into());
        };

        if !info.activatable() {
            return Err(Error::InvalidImage.into());
        }

        if self.is_uploading() {
            return Err(SystemError::Uploading)
        }

        Ok(())
    }

    fn set_info(&self, slot: PartitionSlot, info: PartitionInfo) {
        self.partition(slot, |partition| partition.map(|partition| partition.info.set(info)));
    }
}

impl<'u> System<'u> {
    #[inline]
    fn flash<T>(&self, flash_fun: impl FnOnce(&mut FlashStorage) -> T) -> T {
        self.inner.lock(|system| flash_fun(system.flash.borrow_mut().deref_mut()))
    }

    #[inline]
    fn region<T>(&self, entry: PartitionEntry, region_fun: impl FnOnce(FlashRegion) -> T) -> T {
        self.flash(|flash| region_fun(entry.as_flash_region(flash)))
    }

    #[inline]
    fn nor_region<T>(&self, entry: PartitionEntry, nor_region_fun: impl FnOnce(NorFlashRegion) -> T) -> Result<T, Error> {
        self.region(entry, |mut region| region.as_nor_flash().map(nor_region_fun))
    }

    #[inline]
    fn process<T>(&self, process_fun: impl FnOnce(&mut UploadProcess) -> T) -> T {
        self.inner.lock(|system| process_fun(system.upload_process.borrow_mut().deref_mut()))
    }

    #[inline]
    fn partition<T>(&self, slot: PartitionSlot, partition_fun: impl FnOnce(Option<&Partition>) -> T) -> T {
        self.inner.lock(|system| partition_fun(match slot {
            PartitionSlot::Factory => system.factory.as_ref(),
            PartitionSlot::Current => system.current.as_ref(),
            PartitionSlot::Next => system.next.as_ref(),
        }))
    }

    #[inline]
    fn calculate_erase_distribution(size: usize, max_size: usize) -> Result<([usize; 2], usize), SystemError> {
        static BLOCK_SIZE: usize = FlashStorage::BLOCK_SIZE as _;

        if size > max_size {
            return Err(SystemError::OutOfMemory)
        }

        // block erase
        let mut distribution = [size / BLOCK_SIZE, 0];
        let mut erase_size = distribution[0] * BLOCK_SIZE;

        if size > erase_size && erase_size + BLOCK_SIZE < max_size {
            distribution[0] += 1;
            erase_size += BLOCK_SIZE;
            return Ok((distribution, erase_size));
        }

        // sector erase
        distribution[1] = (size - erase_size).div_ceil(FlashStorage::ERASE_SIZE);
        erase_size += distribution[1] * FlashStorage::ERASE_SIZE;

        if erase_size > max_size {
            return Err(Error::NotSupported.into())
        }

        Ok((distribution, erase_size))
    }
}

impl CompactFormat for SystemError {
    fn compact_format(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        self.fmt(f)
    }
}

impl CompactFormat for UploadProcess {
    fn compact_format(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        stmt_join!(f.write_char('|')?;
            self.state.fmt(f)?;
            self.message.compact_format(f)?;
            self.erase.compact_format(f)?;
            self.write.compact_format(f)?;
        );
        Ok(())
    }
}

impl Partition {
    fn new(entry: PartitionEntry) -> Self {
        Partition { entry, info: Cell::new(PartitionInfo::default()) }
    }
}

impl PartitionInfo {
    const fn activatable(&self) -> bool {
        matches!(self.state, PartitionState::Available | PartitionState::Foreign | PartitionState::Unverified)
    }
}

impl CompactFormat for PartitionInfo {
    fn compact_format(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        stmt_join!(f.write_char('|')?;
            self.state.fmt(f)?;
            self.segment_count.compact_format(f)?;
            self.entry_address.compact_format(f)?;
            self.chip_id.compact_format(f)?;
            self.hash_appended.compact_format(f)?;
            self.secure_version.compact_format(f)?;
            self.version.compact_format(f)?;
            self.project.compact_format(f)?;
            self.time.compact_format(f)?;
            self.date.compact_format(f)?;
            self.sha256.compact_format(f)?;
            self.digest_sha256.compact_format(f)?;
        );
        Ok(())
    }
}

impl<'g> Drop for UploadGuard<'g> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

impl From<Error> for SystemError {
    fn from(error: Error) -> Self {
        SystemError::OTA(error)
    }
}

// Tasks
#[embassy_executor::task]
pub async fn system_task(system: &'static System<'static>) {
    loop {
        system.activate_signal.wait().await;
        let Some(mut count_down) = system.activate_countdown() else {
            continue;
        };

        while count_down > 0 {
            Timer::after_secs(1).await;
            system.activate_countdown.fetch_sub(1, Ordering::Relaxed);
            count_down -= 1;
        }

        // Final guard, prevent invalid signal
        if system.check_activatable().is_ok() {
            esp_hal::system::software_reset();
        }

        system.activate_countdown.store(u32::MAX, Ordering::Relaxed);
    }
}