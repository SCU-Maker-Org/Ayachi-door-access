// Made by Han_feng

use core::cell::{Cell, RefCell};
use embassy_futures::select::{Either3, select3};
use embassy_sync::blocking_mutex::CriticalSectionMutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use esp_hal::__macro_implementation::static_cell::StaticCell;
use esp_hal::gpio::{Level, Output, OutputConfig, OutputPin};
use super::DOOR_OPEN_ONCE_DELAY;

// Enums
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DoorState{
    Lock,
    Open,
}

// Structs
pub struct Door<'d>(CriticalSectionMutex<DoorRaw<'d>>);

struct DoorRaw<'d> {
    state: Cell<DoorState>,
    lock: RefCell<Output<'d>>,
}

pub struct DoorSignal {
    pub open: Signal<CriticalSectionRawMutex, ()>,
    pub open_once: Signal<CriticalSectionRawMutex, ()>,
    pub lock: Signal<CriticalSectionRawMutex, ()>,
}

// Statics
static DOOR: StaticCell<Door> = StaticCell::new();
static DOOR_SIGNAL: StaticCell<DoorSignal> = StaticCell::new();

// Impls
impl<'d> Door<'d> {
    pub fn init(lock: impl OutputPin + 'static) -> &'static Door<'static> {
        DOOR.init(Door(CriticalSectionMutex::new(DoorRaw {
            state: Cell::new(DoorState::Lock),
            lock: RefCell::new(Output::new(lock, Level::Low, OutputConfig::default()))
        })))
    }

    pub fn status(&self) -> DoorState {
        self.0.lock(|door| door.state.get())
    }

    fn open(&self) {
        self.0.lock(|door| {
            door.state.set(DoorState::Open);
            door.lock.borrow_mut().set_high();
        })
    }

    fn lock(&self) {
        self.0.lock(|door| {
            door.state.set(DoorState::Lock);
            door.lock.borrow_mut().set_low();
        })
    }
}

impl DoorSignal {
    pub fn init() -> &'static DoorSignal {
        DOOR_SIGNAL.init(DoorSignal {
            open: Signal::new(),
            open_once: Signal::new(),
            lock: Signal::new(),
        })
    }
}

// Tasks
#[embassy_executor::task]
pub async fn door_task(door: &'static Door<'static>, signal: &'static DoorSignal) {
    'door: loop {
        match select3(
            signal.open.wait(),
            signal.open_once.wait(),
            signal.lock.wait(),
        ).await {
            Either3::First(_) => door.open(),
            Either3::Second(_) if door.status() == DoorState::Lock => {
                door.open();

                loop {
                    match select3(
                        signal.open.wait(),
                        signal.open_once.wait(),
                        embassy_time::with_timeout(embassy_time::Duration::from_millis(DOOR_OPEN_ONCE_DELAY), signal.lock.wait()),
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