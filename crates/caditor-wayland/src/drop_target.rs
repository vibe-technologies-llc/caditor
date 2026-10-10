use std::{
    io,
    ops::RangeInclusive,
    os::fd::AsFd,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use parking_lot::Mutex;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use wayland_client::{
    Connection, Dispatch, DispatchError, EventQueue, Proxy, QueueHandle,
    backend::WaylandError,
    delegate_noop, event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_data_device::{self, WlDataDevice},
        wl_data_device_manager::{DndAction, WlDataDeviceManager},
        wl_data_offer::{self, WlDataOffer},
        wl_registry::{self, WlRegistry},
        wl_seat::WlSeat,
        wl_surface::WlSurface,
    },
};

use crate::{
    connection::{AttachError, WindowConnection},
    reading::{PATIENCE, read_whole},
    tracker::{DropEvent, Entered, Step, Ticket, Tracker},
    uri_list::{URI_LIST, file_paths},
};

const SEAT: &str = "wl_seat";
const SEAT_VERSION: u32 = 1;
const DATA_DEVICE_VERSIONS: RangeInclusive<u32> = 1..=3;
const ACTIONS_SINCE: u32 = 3;
const RELEASE_SINCE: u32 = 2;

#[derive(Debug, thiserror::Error)]
pub enum DropError {
    #[error("the Wayland events for dragged files could not be handled: {0}")]
    Dispatch(#[from] DispatchError),
    #[error("the Wayland connection failed: {0}")]
    Connection(#[from] WaylandError),
}

type Wake = Arc<Mutex<Box<dyn Fn() + Send>>>;

struct Finished {
    ticket: Ticket,
    paths: Option<Vec<PathBuf>>,
}

#[derive(Default)]
struct OfferData {
    lists_files: AtomicBool,
}

struct Seat {
    name: u32,
    device: WlDataDevice,
}

struct State {
    surface: WlSurface,
    manager: WlDataDeviceManager,
    seats: Vec<Seat>,
    tracker: Tracker<WlDataOffer>,
    steps: Vec<Step<WlDataOffer>>,
}

pub struct DropTarget {
    queue: EventQueue<State>,
    state: State,
    finished: Receiver<Finished>,
    finishing: Sender<Finished>,
    wake: Wake,
    local_host: Arc<[u8]>,
    window: WindowConnection,
}

impl DropTarget {
    pub fn attach<W>(
        window: Arc<W>,
        wake: impl Fn() + Send + 'static,
    ) -> Result<Option<Self>, AttachError>
    where
        W: HasWindowHandle + HasDisplayHandle + Send + Sync + 'static,
    {
        let Some(window) = WindowConnection::of_window(window)? else {
            return Ok(None);
        };
        let (globals, queue) = registry_queue_init::<State>(&window.connection)?;
        let handle = queue.handle();
        let manager: WlDataDeviceManager = globals.bind(&handle, DATA_DEVICE_VERSIONS, ())?;
        let seats = globals.contents().with_list(|list| {
            list.iter()
                .filter(|global| global.interface == SEAT)
                .map(|global| global.name)
                .collect::<Vec<_>>()
        });
        let mut state = State {
            surface: window.surface.clone(),
            manager,
            seats: Vec::new(),
            tracker: Tracker::default(),
            steps: Vec::new(),
        };
        for name in seats {
            state.add_seat(globals.registry(), name, &handle);
        }
        let (finishing, finished) = mpsc::channel();
        let target = Self {
            queue,
            state,
            finished,
            finishing,
            wake: Arc::new(Mutex::new(Box::new(wake))),
            local_host: rustix::system::uname().nodename().to_bytes().into(),
            window,
        };
        target.flush()?;
        Ok(Some(target))
    }

    pub fn events(&mut self) -> Result<Vec<DropEvent>, DropError> {
        self.queue.dispatch_pending(&mut self.state)?;
        while let Ok(Finished { ticket, paths }) = self.finished.try_recv() {
            let steps = self.state.tracker.read(ticket, paths);
            self.state.steps.extend(steps);
        }
        let mut events = Vec::new();
        while !self.state.steps.is_empty() {
            for step in std::mem::take(&mut self.state.steps) {
                self.perform(step, &mut events);
            }
        }
        self.flush()?;
        Ok(events)
    }

    fn perform(&mut self, step: Step<WlDataOffer>, events: &mut Vec<DropEvent>) {
        match step {
            Step::Accept { offer, serial } => {
                offer.accept(serial, Some(URI_LIST.to_owned()));
                if offer.version() >= ACTIONS_SINCE {
                    offer.set_actions(DndAction::Copy, DndAction::Copy);
                }
            }
            Step::Refuse { offer, serial } => offer.accept(serial, None),
            Step::Read { offer, ticket } => {
                if let Err(error) = self.start_reading(&offer, ticket) {
                    log::warn!("the list of dragged files could not be asked for: {error}");
                    let steps = self.state.tracker.read(ticket, None);
                    self.state.steps.extend(steps);
                }
            }
            Step::Finish(offer) => {
                if offer.version() >= ACTIONS_SINCE {
                    offer.finish();
                }
                offer.destroy();
            }
            Step::Discard(offer) => offer.destroy(),
            Step::Emit(event) => events.push(event),
        }
    }

    fn start_reading(&self, offer: &WlDataOffer, ticket: Ticket) -> io::Result<()> {
        let (reader, writer) = io::pipe()?;
        offer.receive(URI_LIST.to_owned(), writer.as_fd());
        drop(writer);
        let finishing = self.finishing.clone();
        let wake = Arc::clone(&self.wake);
        let local_host = Arc::clone(&self.local_host);
        thread::Builder::new()
            .name("dragged-files".to_owned())
            .spawn(move || {
                let paths = match read_whole(reader, PATIENCE) {
                    Ok(list) => Some(file_paths(&list, &local_host)),
                    Err(error) => {
                        log::warn!("{error}");
                        None
                    }
                };
                if finishing.send(Finished { ticket, paths }).is_ok() {
                    (wake.lock())();
                }
            })
            .map(drop)
    }

    fn flush(&self) -> Result<(), WaylandError> {
        match self.window.connection.flush() {
            Err(WaylandError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
            flushed => flushed,
        }
    }
}

impl Drop for DropTarget {
    fn drop(&mut self) {
        for offer in self.state.tracker.offers() {
            offer.destroy();
        }
        for seat in &self.state.seats {
            seat.release();
        }
        if let Err(error) = self.flush() {
            log::debug!("the Wayland drop target could not say goodbye: {error}");
        }
    }
}

impl Seat {
    fn release(&self) {
        if self.device.version() >= RELEASE_SINCE {
            self.device.release();
        }
    }
}

impl State {
    fn add_seat(&mut self, registry: &WlRegistry, name: u32, handle: &QueueHandle<Self>) {
        let seat: WlSeat = registry.bind(name, SEAT_VERSION, handle, ());
        let device = self.manager.get_data_device(&seat, handle, ());
        self.seats.push(Seat { name, device });
    }

    fn remove_seat(&mut self, name: u32) {
        self.seats.retain(|seat| {
            let kept = seat.name != name;
            if !kept {
                seat.release();
            }
            kept
        });
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name, interface, ..
            } if interface == SEAT => state.add_seat(registry, name, handle),
            wl_registry::Event::GlobalRemove { name } => state.remove_seat(name),
            _ => {}
        }
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let steps = match event {
            wl_data_device::Event::Enter {
                serial,
                surface,
                id,
                ..
            } => {
                let lists_files = id.as_ref().is_some_and(|offer| {
                    offer
                        .data::<OfferData>()
                        .is_some_and(|data| data.lists_files.load(Ordering::Relaxed))
                });
                let entered = Entered {
                    ours: surface == state.surface,
                    lists_files,
                    serial,
                };
                state.tracker.entered(id, entered)
            }
            wl_data_device::Event::Leave => state.tracker.left(),
            wl_data_device::Event::Drop => state.tracker.dropped(),
            wl_data_device::Event::Selection { id: Some(offer) } => {
                offer.destroy();
                Vec::new()
            }
            _ => Vec::new(),
        };
        state.steps.extend(steps);
    }

    event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, OfferData::default())
    ]);
}

impl Dispatch<WlDataOffer, OfferData> for State {
    fn event(
        _: &mut Self,
        _: &WlDataOffer,
        event: wl_data_offer::Event,
        data: &OfferData,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event
            && mime_type == URI_LIST
        {
            data.lists_files.store(true, Ordering::Relaxed);
        }
    }
}

delegate_noop!(State: ignore WlSeat);
delegate_noop!(State: WlDataDeviceManager);
