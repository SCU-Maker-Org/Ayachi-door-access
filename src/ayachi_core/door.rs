// Made by Han_feng

use super::DOOR_OPEN_ONCE_DELAY;
use core::cell::{Cell, RefCell};
use embassy_futures::select::{Either3, select3};
use embassy_sync::blocking_mutex::CriticalSectionMutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use esp_hal::__macro_implementation::static_cell::StaticCell;
use esp_hal::gpio::{Level, Output, OutputConfig, OutputPin};

// Enums
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DoorState{
    Lock,
    Open,
}

// Structs
pub struct Door<'d> {
    inner: CriticalSectionMutex<DoorRaw<'d>>,
    pub open_signal: Signal<CriticalSectionRawMutex, ()>,
    pub open_once_signal: Signal<CriticalSectionRawMutex, ()>,
    pub lock_signal: Signal<CriticalSectionRawMutex, ()>,
}

struct DoorRaw<'d> {
    state: Cell<DoorState>,
    lock: RefCell<Output<'d>>,
}

// Statics
static DOOR: StaticCell<Door> = StaticCell::new();

// Impls
impl<'d> Door<'d> {
    pub fn init(lock: impl OutputPin + 'static) -> &'static Self {
        DOOR.init(Door {
            inner: CriticalSectionMutex::new(DoorRaw {
                state: Cell::new(DoorState::Lock),
                lock: RefCell::new(Output::new(lock, Level::Low, OutputConfig::default()))
            }),
            open_signal: Signal::new(),
            open_once_signal: Signal::new(),
            lock_signal: Signal::new(),
        })
    }

    pub fn status(&self) -> DoorState {
        self.inner.lock(|door| door.state.get())
    }

    fn open(&self) {
        self.inner.lock(|door| {
            door.state.set(DoorState::Open);
            door.lock.borrow_mut().set_high();
        })
    }

    fn lock(&self) {
        self.inner.lock(|door| {
            door.state.set(DoorState::Lock);
            door.lock.borrow_mut().set_low();
        })
    }
}

// Tasks
#[embassy_executor::task]
pub async fn door_task(door: &'static Door<'static>) {
    'door: loop {
        match select3(
            door.open_signal.wait(),
            door.open_once_signal.wait(),
            door.lock_signal.wait(),
        ).await {
            Either3::First(_) => door.open(),
            Either3::Second(_) if door.status() == DoorState::Lock => {
                door.open();

                loop {
                    match select3(
                        door.open_signal.wait(),
                        door.open_once_signal.wait(),
                        embassy_time::with_timeout(embassy_time::Duration::from_millis(DOOR_OPEN_ONCE_DELAY), door.lock_signal.wait()),
                    ).await {
                        Either3::First(_) => continue 'door,
                        Either3::Second(_) => continue,
                        Either3::Third(_) => break,
                    }
                }

                door.lock();
            }
            Either3::Third(_) => door.lock(),
            _ => {}
        }
    }
}