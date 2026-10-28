//! Central role for the client: find a server, connect, discover the
//! service, read characteristics, open the L2CAP channel.

use std::{
    io,
    sync::{mpsc, Mutex},
    time::{Duration, Instant},
};

use blessh_core::protocol::SERVICE_UUID;
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use objc2::{
    define_class, msg_send, rc::Retained, runtime::AnyObject, runtime::ProtocolObject, AnyThread,
    DefinedClass, Message,
};
use objc2_core_bluetooth::CBManager;
use objc2_core_bluetooth::{
    CBAdvertisementDataLocalNameKey, CBCentralManager, CBCentralManagerDelegate,
    CBCentralManagerScanOptionAllowDuplicatesKey, CBCharacteristic, CBL2CAPChannel, CBManagerState,
    CBPeripheral, CBPeripheralDelegate, CBPeripheralState, CBService,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSNumber, NSObject, NSObjectProtocol, NSString, NSUUID,
};

use super::{cb_uuid, error_text, stream::L2capStream, uuid_of, Shared};

enum Event {
    State(CBManagerState),
    Discovered {
        peripheral: Shared<CBPeripheral>,
        local_name: Option<String>,
        gap_name: Option<String>,
        rssi: i64,
    },
    /// Link events name the peripheral: the central may also see other
    /// peripherals it looked at come and go.
    Connected(String),
    ConnectFailed(String, String),
    Disconnected(String, Option<String>),
    Services(Result<(), String>),
    Characteristics(Result<(), String>),
    Value {
        uuid: String,
        value: Result<Vec<u8>, String>,
    },
    Channel(Result<L2capStream, String>),
}

struct Ivars {
    events: Mutex<mpsc::Sender<Event>>,
}

fn result(error: Option<&NSError>) -> Result<(), String> {
    match error {
        Some(e) => Err(error_text(Some(e))),
        None => Ok(()),
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements.
    #[unsafe(super(NSObject))]
    #[name = "BlesshCentralDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl CBCentralManagerDelegate for Delegate {
        #[unsafe(method(centralManagerDidUpdateState:))]
        fn did_update_state(&self, central: &CBCentralManager) {
            // SAFETY: no preconditions.
            let state = unsafe { central.state() };
            log::debug!("Bluetooth state: {state:?}");
            self.send(Event::State(state));
        }

        #[unsafe(method(centralManager:didDiscoverPeripheral:advertisementData:RSSI:))]
        fn did_discover(
            &self,
            _central: &CBCentralManager,
            peripheral: &CBPeripheral,
            advertisement_data: &NSDictionary<NSString, AnyObject>,
            rssi: &NSNumber,
        ) {
            // SAFETY: a CoreBluetooth constant; the dictionary holds strings
            // under the local name key.
            let local_name =
                unsafe { advertisement_data.objectForKey(CBAdvertisementDataLocalNameKey) };
            let local_name =
                local_name.and_then(|v| v.downcast_ref::<NSString>().map(|s| s.to_string()));
            // SAFETY: no preconditions.
            let gap_name = unsafe { peripheral.name() }.map(|n| n.to_string());
            self.send(Event::Discovered {
                peripheral: Shared(peripheral.retain()),
                local_name,
                gap_name,
                rssi: rssi.integerValue() as i64,
            });
        }

        #[unsafe(method(centralManager:didConnectPeripheral:))]
        fn did_connect(&self, _central: &CBCentralManager, peripheral: &CBPeripheral) {
            self.send(Event::Connected(peripheral_id(peripheral)));
        }

        #[unsafe(method(centralManager:didFailToConnectPeripheral:error:))]
        fn did_fail_to_connect(
            &self,
            _central: &CBCentralManager,
            peripheral: &CBPeripheral,
            error: Option<&NSError>,
        ) {
            self.send(Event::ConnectFailed(
                peripheral_id(peripheral),
                error_text(error),
            ));
        }

        #[unsafe(method(centralManager:didDisconnectPeripheral:error:))]
        fn did_disconnect(
            &self,
            _central: &CBCentralManager,
            peripheral: &CBPeripheral,
            error: Option<&NSError>,
        ) {
            self.send(Event::Disconnected(
                peripheral_id(peripheral),
                error.map(|e| error_text(Some(e))),
            ));
        }
    }

    unsafe impl CBPeripheralDelegate for Delegate {
        #[unsafe(method(peripheral:didDiscoverServices:))]
        fn did_discover_services(&self, _peripheral: &CBPeripheral, error: Option<&NSError>) {
            self.send(Event::Services(result(error)));
        }

        #[unsafe(method(peripheral:didDiscoverCharacteristicsForService:error:))]
        fn did_discover_characteristics(
            &self,
            _peripheral: &CBPeripheral,
            _service: &CBService,
            error: Option<&NSError>,
        ) {
            self.send(Event::Characteristics(result(error)));
        }

        #[unsafe(method(peripheral:didUpdateValueForCharacteristic:error:))]
        fn did_update_value(
            &self,
            _peripheral: &CBPeripheral,
            characteristic: &CBCharacteristic,
            error: Option<&NSError>,
        ) {
            // SAFETY: no preconditions.
            let uuid = uuid_of(&*unsafe { characteristic.UUID() });
            let value = match error {
                Some(e) => Err(error_text(Some(e))),
                // SAFETY: no preconditions.
                None => Ok(unsafe { characteristic.value() }
                    .map(|d| d.to_vec())
                    .unwrap_or_default()),
            };
            self.send(Event::Value { uuid, value });
        }

        #[unsafe(method(peripheral:didOpenL2CAPChannel:error:))]
        fn did_open_channel(
            &self,
            _peripheral: &CBPeripheral,
            channel: Option<&CBL2CAPChannel>,
            error: Option<&NSError>,
        ) {
            let r = match (channel, error) {
                (Some(c), None) => {
                    L2capStream::spawn(Shared(c.retain())).map_err(|e| e.to_string())
                }
                (_, e) => Err(error_text(e)),
            };
            self.send(Event::Channel(r));
        }
    }
);

impl Delegate {
    fn new(events: mpsc::Sender<Event>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars {
            events: Mutex::new(events),
        });
        // SAFETY: NSObject's init has no preconditions.
        unsafe { msg_send![super(this), init] }
    }

    fn send(&self, event: Event) {
        let _ = self.ivars().events.lock().unwrap().send(event);
    }
}

/// A server seen while scanning.
pub struct Found {
    pub peripheral: Shared<CBPeripheral>,
    /// CoreBluetooth's identifier for the device (macOS shows no addresses).
    pub id: String,
    /// The name in the advertisement (scan response), once received.
    pub local_name: Option<String>,
    /// The device name CoreBluetooth has cached for the device, which may
    /// be an older or another one than it advertises.
    pub gap_name: Option<String>,
    pub rssi: i64,
}

impl Found {
    /// The name to show: the advertised one, else the cached one.
    pub fn name(&self) -> Option<&str> {
        self.local_name.as_deref().or(self.gap_name.as_deref())
    }
}

/// The client's CoreBluetooth central manager with a synchronous API: each
/// call drives the manager and waits for its delegate's answer.
pub struct Central {
    manager: Shared<CBCentralManager>,
    delegate: Shared<Delegate>,
    events: mpsc::Receiver<Event>,
    _queue: DispatchRetained<DispatchQueue>,
    /// The peripheral the last connect was for.
    current: Mutex<String>,
}

/// Whether the peripheral is connected, a moment after an event that
/// suggested otherwise.
fn still_connected(p: &CBPeripheral) -> bool {
    std::thread::sleep(Duration::from_millis(300));
    // SAFETY: no preconditions.
    (unsafe { p.state() }) == CBPeripheralState::Connected
}

fn peripheral_id(p: &CBPeripheral) -> String {
    // SAFETY: no preconditions.
    unsafe { p.identifier().UUIDString() }
        .to_string()
        .to_lowercase()
}

fn timeout_error(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, format!("{what} timed out"))
}

impl Central {
    /// Creates the manager and waits until Bluetooth is on.
    pub fn new(timeout: Duration) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let delegate = Delegate::new(tx);
        let queue = DispatchQueue::new("blessh.central", DispatchQueueAttr::SERIAL);
        // SAFETY: the delegate outlives the manager (both live in this struct).
        let manager = unsafe {
            CBCentralManager::initWithDelegate_queue(
                CBCentralManager::alloc(),
                Some(ProtocolObject::from_ref(&*delegate)),
                Some(&queue),
            )
        };
        let central = Self {
            manager: Shared(manager),
            delegate: Shared(delegate),
            events: rx,
            _queue: queue,
            current: Mutex::new(String::new()),
        };
        // SAFETY: no preconditions.
        log::debug!("Bluetooth authorization: {:?}", unsafe {
            CBManager::authorization_class()
        });
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match central.events.recv_timeout(left) {
                Ok(Event::State(s)) => match s {
                    CBManagerState::PoweredOn => return Ok(central),
                    CBManagerState::PoweredOff => return Err(io::Error::other("Bluetooth is off")),
                    CBManagerState::Unauthorized => {
                        return Err(io::Error::other(
                            "Bluetooth access is not allowed for this program (System Settings, Privacy & Security, Bluetooth)",
                        ))
                    }
                    CBManagerState::Unsupported => {
                        return Err(io::Error::other("this machine has no Bluetooth LE"))
                    }
                    _ => {}
                },
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, super::not_powered_on()))
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(io::Error::other("Bluetooth manager gone")),
            }
        }
    }

    /// Waits for the delegate event that `pick` accepts, or fails on a
    /// disconnect, a Bluetooth state change or the timeout.
    fn wait<T>(
        &self,
        what: &str,
        timeout: Duration,
        mut pick: impl FnMut(Event) -> Option<io::Result<T>>,
    ) -> io::Result<T> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(left) {
                Ok(Event::State(s)) if s != CBManagerState::PoweredOn => {
                    return Err(io::Error::other("Bluetooth went away"))
                }
                Ok(Event::Disconnected(id, e)) if id == *self.current.lock().unwrap() => {
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionReset,
                        format!(
                            "disconnected: {}",
                            e.unwrap_or_else(|| "by the peer".into())
                        ),
                    ))
                }
                Ok(ev) => {
                    if let Some(r) = pick(ev) {
                        return r;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => return Err(timeout_error(what)),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(io::Error::other("Bluetooth manager gone"))
                }
            }
        }
    }

    /// blessh servers this Mac is connected to already (by any process):
    /// such a server may not advertise while the link is up.
    pub fn connected(&self) -> Vec<Found> {
        let uuids = NSArray::from_retained_slice(&[cb_uuid(&SERVICE_UUID)]);
        // SAFETY: no preconditions.
        let list = unsafe {
            self.manager
                .retrieveConnectedPeripheralsWithServices(&uuids)
        };
        list.iter()
            .map(|p| Found {
                // SAFETY: no preconditions.
                id: unsafe { p.identifier().UUIDString() }
                    .to_string()
                    .to_lowercase(),
                // SAFETY: no preconditions.
                gap_name: unsafe { p.name() }.map(|n| n.to_string()),
                local_name: None,
                rssi: 0,
                peripheral: Shared(p),
            })
            .collect()
    }

    /// A peripheral this Mac has seen before, by CoreBluetooth identifier:
    /// it can be connected without scanning.
    pub fn known(&self, id: &str) -> Option<Found> {
        let uuid = NSUUID::from_string(&NSString::from_str(id))?;
        let ids = NSArray::from_retained_slice(&[uuid]);
        // SAFETY: no preconditions.
        let list = unsafe { self.manager.retrievePeripheralsWithIdentifiers(&ids) };
        let p = list.iter().next()?;
        Some(Found {
            id: peripheral_id(&p),
            // SAFETY: no preconditions.
            gap_name: unsafe { p.name() }.map(|n| n.to_string()),
            local_name: None,
            rssi: 0,
            peripheral: Shared(p),
        })
    }

    /// Scans for servers until `found` accepts one or the timeout passes.
    pub fn scan(
        &self,
        timeout: Duration,
        mut found: impl FnMut(&Found) -> bool,
    ) -> io::Result<Option<Found>> {
        let uuids = NSArray::from_retained_slice(&[cb_uuid(&SERVICE_UUID)]);
        // SAFETY: no preconditions.
        // Every advertisement, not only the first: the first may come
        // before the scan response that carries the name.
        // SAFETY: a CoreBluetooth constant.
        let key = unsafe { CBCentralManagerScanOptionAllowDuplicatesKey };
        let yes = NSNumber::new_bool(true);
        let yes_obj: &AnyObject = &yes;
        let options = NSDictionary::<NSString, AnyObject>::from_slices(&[key], &[yes_obj]);
        // SAFETY: no preconditions.
        unsafe {
            self.manager
                .scanForPeripheralsWithServices_options(Some(&uuids), Some(&options))
        };
        let deadline = Instant::now() + timeout;
        let result = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break Ok(None);
            }
            match self.events.recv_timeout(left) {
                Ok(Event::Discovered {
                    peripheral,
                    local_name,
                    gap_name,
                    rssi,
                }) => {
                    // SAFETY: no preconditions.
                    let id = unsafe { peripheral.identifier().UUIDString() }
                        .to_string()
                        .to_lowercase();
                    let f = Found {
                        peripheral,
                        id,
                        local_name,
                        gap_name,
                        rssi,
                    };
                    if found(&f) {
                        break Ok(Some(f));
                    }
                }
                Ok(Event::State(s)) if s != CBManagerState::PoweredOn => {
                    break Err(io::Error::other("Bluetooth went away"));
                }
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => break Ok(None),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break Err(io::Error::other("Bluetooth manager gone"))
                }
            }
        };
        // SAFETY: no preconditions.
        unsafe { self.manager.stopScan() };
        result
    }

    pub fn connect(&self, peripheral: &Shared<CBPeripheral>, timeout: Duration) -> io::Result<()> {
        // SAFETY: the delegate outlives the peripheral's use (both live in
        // this struct or the caller's Found).
        let id = peripheral_id(peripheral);
        *self.current.lock().unwrap() = id.clone();
        unsafe {
            peripheral.setDelegate(Some(ProtocolObject::from_ref(&*self.delegate.0)));
            self.manager.connectPeripheral_options(peripheral, None);
        }
        self.wait("connecting", timeout, |ev| match ev {
            Event::Connected(c) if c == id => Some(Ok(())),
            Event::ConnectFailed(c, e) if c == id => {
                Some(Err(io::Error::other(format!("connect failed: {e}"))))
            }
            _ => None,
        })
    }

    /// Discovers the blessh service and returns its characteristics by UUID
    /// (lower-case).
    pub fn discover(
        &self,
        peripheral: &Shared<CBPeripheral>,
        timeout: Duration,
    ) -> io::Result<Vec<(String, Shared<CBCharacteristic>)>> {
        let service_uuid = SERVICE_UUID.to_string();
        let uuids = NSArray::from_retained_slice(&[cb_uuid(&SERVICE_UUID)]);
        // SAFETY: no preconditions.
        unsafe { peripheral.discoverServices(Some(&uuids)) };
        self.wait("service discovery", timeout, |ev| match ev {
            Event::Services(r) => Some(r.map_err(io::Error::other)),
            _ => None,
        })?;
        // SAFETY: no preconditions.
        let services = unsafe { peripheral.services() }.unwrap_or_default();
        let service = services
            .iter()
            // SAFETY: no preconditions.
            .find(|s| uuid_of(&*unsafe { s.UUID() }) == service_uuid)
            .map(|s| s.retain())
            .ok_or_else(|| io::Error::other("device does not provide the blessh service"))?;
        // SAFETY: no preconditions.
        unsafe { peripheral.discoverCharacteristics_forService(None, &service) };
        self.wait("characteristic discovery", timeout, |ev| match ev {
            Event::Characteristics(r) => Some(r.map_err(io::Error::other)),
            _ => None,
        })?;
        // SAFETY: no preconditions.
        let chars = unsafe { service.characteristics() }.unwrap_or_default();
        Ok(chars
            .iter()
            // SAFETY: no preconditions.
            .map(|c| (uuid_of(&*unsafe { c.UUID() }), Shared(c.retain())))
            .collect())
    }

    pub fn read(
        &self,
        peripheral: &Shared<CBPeripheral>,
        characteristic: &Shared<CBCharacteristic>,
        timeout: Duration,
    ) -> io::Result<Vec<u8>> {
        // SAFETY: no preconditions.
        let want = uuid_of(&*unsafe { characteristic.UUID() });
        // SAFETY: no preconditions.
        unsafe { peripheral.readValueForCharacteristic(characteristic) };
        self.wait("reading a characteristic", timeout, |ev| match ev {
            Event::Value { uuid, value } if uuid == want => Some(value.map_err(io::Error::other)),
            _ => None,
        })
    }

    pub fn open_channel(
        &self,
        peripheral: &Shared<CBPeripheral>,
        psm: u16,
        timeout: Duration,
    ) -> io::Result<L2capStream> {
        // SAFETY: no preconditions.
        unsafe { peripheral.openL2CAPChannel(psm) };
        self.wait("opening the L2CAP channel", timeout, |ev| match ev {
            Event::Channel(r) => Some(r.map_err(io::Error::other)),
            _ => None,
        })
    }

    /// Waits until the link to the connected peripheral goes down (Some,
    /// with the reason) or `stop` is set (None). An open channel does not
    /// always report a lost link on its streams.
    pub fn wait_disconnected(
        &self,
        peripheral: &Shared<CBPeripheral>,
        stop: &std::sync::atomic::AtomicBool,
    ) -> Option<String> {
        loop {
            match self.events.recv_timeout(Duration::from_millis(200)) {
                // A disconnect another process asked for may be reported
                // while the link is up again for this one, and the manager
                // passes through other states for a moment: what counts is
                // whether the peripheral is still connected.
                Ok(Event::Disconnected(id, e)) if id == *self.current.lock().unwrap() => {
                    if still_connected(peripheral) {
                        log::debug!("ignoring a disconnect report: the link is up");
                        continue;
                    }
                    return Some(e.unwrap_or_else(|| "disconnected".into()));
                }
                Ok(Event::State(s)) if s != CBManagerState::PoweredOn => {
                    if still_connected(peripheral) {
                        log::debug!("ignoring Bluetooth state {s:?}: the link is up");
                        continue;
                    }
                    return Some(format!("Bluetooth state {s:?}"));
                }
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Some("Bluetooth manager gone".into())
                }
            }
            if stop.load(std::sync::atomic::Ordering::SeqCst) {
                return None;
            }
        }
    }

    pub fn disconnect(&self, peripheral: &Shared<CBPeripheral>) {
        // SAFETY: no preconditions.
        unsafe { self.manager.cancelPeripheralConnection(peripheral) };
    }
}
