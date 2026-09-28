// Made by Han_feng

use super::{SLOTS, factory_request_reset};
use super::utils::{ArrayResult, CompactFormat, TextBytes};
use crate::stmt_join;
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
use esp_bootloader_esp_idf::partitions::{read_partition_table, AppPartitionSubType, DataPartitionSubType, Error, FlashRegion, NorFlashRegion, PartitionEntry, PartitionType, PARTITION_TABLE_MAX_LEN};
use esp_hal::__macro_implementation::static_cell::StaticCell;
use esp_hal::peripherals::FLASH;
use esp_storage::FlashStorage;
use pbkdf2::sha2::{Digest, Sha256};

// Enums
#[derive(Copy, Clone, Debug, defmt::Format)]
pub enum SystemError {
    Uploading,
    OutOfMemory,
    Incomplete,
    WriteFailure,
    OTADataInvalid,
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
    Unchecked,
    ReadFailed,
    Empty,
    Unknown,
    Broken,
    Unverified,
    Foreign,
    Available
}

// Structs
pub struct System<'u> {
    uploading: AtomicBool,
    booted: Option<usize>,
    ota_entry: Option<PartitionEntry>,
    activate_countdown: AtomicU32,
    activate_signal: Signal<CriticalSectionRawMutex, usize>,
    inner: CriticalSectionMutex<SystemRaw<'u>>
}

struct SystemRaw<'u> {
    flash: RefCell<FlashStorage<'u>>,
    upload_process: RefCell<UploadProcess>,
    partition_slots: [Option<Partition>; SLOTS],
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
    pub name: TextBytes<16>,
    pub capacity: u32,
    pub size: u32,

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

        let mut booted = None;
        let mut ota_entry = None;
        let mut partition_slots = [const { None }; SLOTS];
        if let Ok(table) = read_partition_table(&mut flash, &mut buffer) {
            booted = table.booted_partition().ok().flatten().and_then(|booted_entry| match booted_entry.partition_type() {
                PartitionType::App(subtype) => Self::get_index(subtype).ok(),
                _ => None
            });

            for entry in table.iter() {
                match entry.partition_type() {
                    PartitionType::App(subtype) if let Ok(index) = Self::get_index(subtype) => {
                        partition_slots[index] = Some(Partition::new(entry));
                    }
                    PartitionType::Data(DataPartitionSubType::Ota) => {
                        ota_entry = Some(entry);
                    }
                    _ => ()
                }
            }
        }

        // check the ota data
        if let Some(entry) = ota_entry && let Ok(mut updater) = OtaUpdater::new(&mut flash, &mut buffer) && let Ok(mut data) = updater.ota_data() && let Err(error) = data.current_app_partition() {
            defmt::warn!("OTA data is broken: {}", error);
            let mut ota_region = entry.as_flash_region(&mut flash);
            match ota_region.erase(0, ota_region.capacity() as u32) {
                Ok(_) => defmt::info!("OTA data erased, and it will return to factory on the next restart if nothing has been activated"),
                Err(_) => {
                    defmt::warn!("Failed to erase the OTA data, and the function of activation will be disabled");
                    ota_entry = None;
                },
            }
        }

        let system = System {
            uploading: AtomicBool::new(false),
            booted, ota_entry,
            activate_countdown: AtomicU32::new(u32::MAX),
            activate_signal: Signal::new(),
            inner: CriticalSectionMutex::new(SystemRaw {
                flash: RefCell::new(flash),
                upload_process: RefCell::new(UploadProcess::default()),
                partition_slots
            })
        };

        for index in 0..System::slots() {
            match system.inspect_partition(index).await {
                Ok(info) => defmt::info!("Partition state({}): {}", index, info.state),
                Err(error) => defmt::warn!("Failed to inspect partition({}): {}", index, error),
            }
        }

        SYSTEM.init(system)
    }

    pub async fn upload<F, E>(&self, index: usize, file_size: usize, file_stream: F) -> Result<(), SystemError>
    where
        F: AsyncFnMut(&mut [u8]) -> Result<usize, E>,
        E: Into<SystemError>
    {
        if file_size == 0 || file_size % FlashStorage::WRITE_SIZE != 0 || index >= SLOTS {
            return Err(Error::NotSupported.into());
        }

        if index <= 0 || self.booted.is_some_and(|booted| booted == index) {
           return Err(Error::Invalid.into());
        }

        if self.uploading.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            return Err(SystemError::Uploading);
        }

        let _guard = UploadGuard(&self.uploading);

        self.process(|process| {
            process.state = UploadState::Received;
            process.message = None;
            process.erase = [0; 2];
            process.write = [0; 2];
        });

        if let Err(error) = self.system_upload(index, file_size, file_stream).await && let Some(entry) = self.inner.lock(|system| {
            let mut process = system.upload_process.borrow_mut();
            process.message = Some(error);
            (process.state != UploadState::Received).then_some(system.partition_slots[index].as_ref().map(|next| next.entry)).flatten()
        }) {
            self.set_partition_info(index, self.inspect(entry, None).await)?;
        }

        Ok(())
    }

    pub fn activate(&self, index: usize) -> Result<(), SystemError> {
        static COUNTDOWN_SECONDS: u32 = 5;

        self.check_activatable(index)?;

        let Some(ota_entry) = self.ota_entry else {
            return Err(SystemError::OTADataInvalid);
        };

        self.flash(|flash| -> Result<(), SystemError> {
            let mut ota_region = ota_entry.as_flash_region(flash);

            if index == 0 {
                // If reset to factory, we have to erase all the ota data and keep it clean
                ota_region.erase(0, ota_region.capacity() as u32)?;
                return Ok(());
            }

            // erase the ota data
            let erase_offset = match [0, 1].map(|i| {
                let mut sequence = [0; 4];
                ota_region.read(i * FlashStorage::SECTOR_SIZE, &mut sequence)?;
                Ok::<_, Error>(u32::from_le_bytes(sequence))
            }).collect_result()? {
                [u32::MAX, u32::MAX] => 0,
                [u32::MAX, _] => 0,
                [_, u32::MAX] => 1,
                [a, b] if a > b => 1,
                _ => 0,
            } * FlashStorage::SECTOR_SIZE;

            ota_region.erase(erase_offset, erase_offset + FlashStorage::SECTOR_SIZE)?;

            // select the certain partition
            let mut buffer = [0; PARTITION_TABLE_MAX_LEN];
            let mut updater = OtaUpdater::new(flash, &mut buffer)?;

            updater.ota_data()?.set_current_app_partition(Self::get_type(index)?)?;
            updater.set_current_ota_state(OtaImageState::New)?;

            Ok(())
        })?;

        self.activate_countdown.store(COUNTDOWN_SECONDS, Ordering::Relaxed);
        self.activate_signal.signal(index);

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

    pub fn get_partition_info(&self, index: usize) -> Result<PartitionInfo, SystemError> {
        self.partition(index, |partition| partition.info.get())
    }

    pub async fn inspect_partition(&self, index: usize) -> Result<PartitionInfo, SystemError> {
        let entry = self.partition(index, |partition| partition.entry)?;
        let info = self.inspect(entry, None).await;
        self.set_partition_info(index, info)?;
        Ok(info)
    }

    pub fn booted(&self) -> Option<usize> {
        self.booted
    }

    pub fn slots() -> usize {
        SLOTS
    }
}

impl<'u> System<'u> {
    async fn system_upload<F, E>(&self, index: usize, file_size: usize, mut file_stream: F) -> Result<(), SystemError>
    where
        F: AsyncFnMut(&mut [u8]) -> Result<usize, E>,
        E: Into<SystemError>
    {
        let entry = self.partition(index, |partition| partition.entry)?;

        // Erase the partition
        let (distribution, total) = Self::calculate_erase_distribution(file_size, self.region(entry, |target| target.capacity()))?;
        let mut erased = 0;
        self.inner.lock(|system| {
            let mut process = system.upload_process.borrow_mut();
            process.state = UploadState::Erasing;
            process.erase = [erased, total as u32];

            system.partition_slots[index].as_ref().map(|next| next.info.set(PartitionInfo::default()));
        });

        for _ in 0..distribution[0] {
            // block erase
            self.region(entry, |mut target| target.erase(erased, erased + FlashStorage::BLOCK_SIZE))?;
            erased += FlashStorage::BLOCK_SIZE;
            self.process(|process| process.erase[0] = erased);
            yield_now().await;
        }

        for _ in 0..distribution[1] {
            // sector erase
            self.region(entry, |mut target| target.erase(erased, erased + FlashStorage::ERASE_SIZE as u32))?;
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
                        self.nor_region(entry, |mut target| target.write(written, &write_buffer[..buffer_length]))??;
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
                        self.nor_region(entry, |mut target| target.write(written, &write_buffer))??;
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

        let partition_info = self.inspect(entry, Some(file_size)).await;

        self.inner.lock(|system| {
            system.upload_process.borrow_mut().state = partition_info.activatable().then_some(UploadState::Ready).unwrap_or(UploadState::Failed);
            system.partition_slots[index].as_ref().map(|next| next.info.set(partition_info));
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

        result.capacity = entry.len();
        let max_length = result.name.len().min(entry.label().len());
        result.name[..max_length].copy_from_slice(&entry.label()[..max_length]);

        let mut buffer = [0; PARTITION_HEADER_LENGTH];
        if self.region(entry, |mut region| region.read(0, &mut buffer)).is_err() {
            result.state = PartitionState::ReadFailed;
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

        result.size = image_length;

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

    fn check_activatable(&self, index: usize) -> Result<(), SystemError> {
        if !self.get_partition_info(index)?.activatable() {
            return Err(Error::InvalidImage.into());
        }

        if self.is_uploading() {
            return Err(SystemError::Uploading)
        }

        Ok(())
    }

    fn set_partition_info(&self, index: usize, info: PartitionInfo) -> Result<(), SystemError> {
        self.partition(index, |partition| partition.info.set(info))
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
    fn partition<T>(&self, index: usize, partition_fun: impl FnOnce(&Partition) -> T) -> Result<T, SystemError> {
        self.inner.lock(|system| Ok(partition_fun(system.partition_slots.get(index).ok_or(Error::NotSupported)?.as_ref().ok_or(Error::Invalid)?)))
    }

    #[inline]
    fn get_index(partition_type: AppPartitionSubType) -> Result<usize, SystemError> {
        Ok(match partition_type {
            AppPartitionSubType::Factory => 0,
            AppPartitionSubType::Ota0 => 1,
            AppPartitionSubType::Ota1 => 2,
            AppPartitionSubType::Ota2 => 3,
            AppPartitionSubType::Ota3 => 4,
            AppPartitionSubType::Ota4 => 5,
            AppPartitionSubType::Ota5 => 6,
            AppPartitionSubType::Ota6 => 7,
            AppPartitionSubType::Ota7 => 8,
            AppPartitionSubType::Ota8 => 9,
            AppPartitionSubType::Ota9 => 10,
            AppPartitionSubType::Ota10 => 11,
            AppPartitionSubType::Ota11 => 12,
            AppPartitionSubType::Ota12 => 13,
            AppPartitionSubType::Ota13 => 14,
            AppPartitionSubType::Ota14 => 15,
            AppPartitionSubType::Ota15 => 16,
            _ => return Err(Error::NotSupported.into()),
        })
    }

    #[inline]
    pub fn get_index_by_str(string: impl AsRef<str>) -> Result<usize, SystemError> {
        Ok(match string.as_ref() {
            "factory" => 0,
            idx if let Ok(index) = idx.parse::<usize>() && index < 16 => index + 1,
            _ => return Err(Error::NotSupported.into()),
        })
    }

    #[inline]
    fn get_type(index: usize) -> Result<AppPartitionSubType, SystemError> {
        Ok(match index {
            0 => AppPartitionSubType::Factory,
            1 => AppPartitionSubType::Ota0,
            2 => AppPartitionSubType::Ota1,
            3 => AppPartitionSubType::Ota2,
            4 => AppPartitionSubType::Ota3,
            5 => AppPartitionSubType::Ota4,
            6 => AppPartitionSubType::Ota5,
            7 => AppPartitionSubType::Ota6,
            8 => AppPartitionSubType::Ota7,
            9 => AppPartitionSubType::Ota8,
            10 => AppPartitionSubType::Ota9,
            11 => AppPartitionSubType::Ota10,
            12 => AppPartitionSubType::Ota11,
            13 => AppPartitionSubType::Ota12,
            14 => AppPartitionSubType::Ota13,
            15 => AppPartitionSubType::Ota14,
            16 => AppPartitionSubType::Ota15,
            _ => return Err(Error::NotSupported.into()),
        })
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
            self.name.compact_format(f)?;
            self.capacity.compact_format(f)?;
            self.size.compact_format(f)?;
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
        let index = system.activate_signal.wait().await;
        let Some(mut count_down) = system.activate_countdown() else {
            continue;
        };

        while count_down > 0 {
            Timer::after_secs(1).await;
            system.activate_countdown.fetch_sub(1, Ordering::Relaxed);
            count_down -= 1;
        }

        // Final guard, prevent invalid signal
        if system.check_activatable(index).is_ok() {
            factory_request_reset();
            esp_hal::system::software_reset();
        }

        system.activate_countdown.store(u32::MAX, Ordering::Relaxed);
    }
}