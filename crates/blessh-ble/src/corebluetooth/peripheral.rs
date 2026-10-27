//! Peripheral role for the server: advertise the service, answer INFO and
//! PSM reads, publish the L2CAP channel and hand over every channel a
//! central opens. The GATT stream (RX/TX) is not served on macOS: writes
//! are refused, so GATT-only clients fail fast.

use std::{
    io,
    sync::{mpsc, Mutex},
    time::{Duration, Instant},
};

use blessh_core::protocol::{self, INFO_UUID, PSM_UUID, RX_UUID, SERVICE_UUID, TX_UUID};
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use objc2::{
    define_class, msg_send, rc::Retained, runtime::AnyObject, runtime::ProtocolObject, AnyThread,
    DefinedClass, Message,
};
use objc2_core_bluetooth::{
    CBATTError, CBATTRequest, CBAdvertisementDataLocalNameKey, CBAdvertisementDataServiceUUIDsKey,
    CBAttributePermissions, CBCharacteristic, CBCharacteristicProperties, CBL2CAPChannel,
    CBManagerState, CBMutableCharacteristic, CBMutableService, CBPeripheralManager,
    CBPeripheralManagerDelegate, CBService, CBL2CAPPSM,
};
use objc2_foundation::{
    NSArray, NSData, NSDictionary, NSError, NSObject, NSObjectProtocol, NSString,
};

use super::{cb_uuid, error_text, stream::L2capStream, uuid_of, Shared};

/// A channel a central opened: one session.
pub struct Session {
    pub stream: L2capStream,
    /// CoreBluetooth's identifier for the central.
    pub central: String,
}

enum Event {
    State(CBManagerState),
    Published(Result<u16, String>),
    ServiceAdded(Result<(), String>),
    Advertising(Result<(), String>),
    Channel(Result<Session, String>),
}

struct Ivars {
    events: Mutex<mpsc::Sender<Event>>,
    name: String,
    psm: Mutex<Option<u16>>,
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
    #[name = "BlesshPeripheralDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl CBPeripheralManagerDelegate for Delegate {
        #[unsafe(method(peripheralManagerDidUpdateState:))]
        fn did_update_state(&self, manager: &CBPeripheralManager) {
            // SAFETY: no preconditions.
            self.send(Event::State(unsafe { manager.state() }));
        }

        #[unsafe(method(peripheralManager:didPublishL2CAPChannel:error:))]
        fn did_publish(
            &self,
            _manager: &CBPeripheralManager,
            psm: CBL2CAPPSM,
            error: Option<&NSError>,
        ) {
            let r = match error {
                Some(e) => Err(error_text(Some(e))),
                None => {
                    *self.ivars().psm.lock().unwrap() = Some(psm);
                    Ok(psm)
                }
            };
            self.send(Event::Published(r));
        }

        #[unsafe(method(peripheralManager:didAddService:error:))]
        fn did_add_service(
            &self,
            _manager: &CBPeripheralManager,
            _service: &CBService,
            error: Option<&NSError>,
        ) {
            self.send(Event::ServiceAdded(result(error)));
        }

        #[unsafe(method(peripheralManagerDidStartAdvertising:error:))]
        fn did_start_advertising(&self, _manager: &CBPeripheralManager, error: Option<&NSError>) {
            self.send(Event::Advertising(result(error)));
        }

        #[unsafe(method(peripheralManager:didReceiveReadRequest:))]
        fn did_receive_read(&self, manager: &CBPeripheralManager, request: &CBATTRequest) {
            // SAFETY: no preconditions.
            let uuid = uuid_of(&*unsafe { request.characteristic().UUID() });
            let psm = *self.ivars().psm.lock().unwrap();
            let value = if uuid == INFO_UUID.to_string() {
                // SAFETY: no preconditions.
                let mtu =
                    unsafe { request.central().maximumUpdateValueLength() }.min(65532) as u16 + 3;
                protocol::info(&self.ivars().name, mtu, false, psm).into_bytes()
            } else if uuid == PSM_UUID.to_string() {
                psm.unwrap_or(0).to_le_bytes().to_vec()
            } else {
                // SAFETY: the request came from this manager.
                unsafe {
                    manager.respondToRequest_withResult(request, CBATTError::ReadNotPermitted)
                };
                return;
            };
            // SAFETY: no preconditions.
            let offset = unsafe { request.offset() }.min(u16::MAX as usize) as u16;
            match protocol::read_at(&value, offset) {
                Some(part) => {
                    // SAFETY: the request came from this manager.
                    unsafe {
                        request.setValue(Some(&NSData::with_bytes(part)));
                        manager.respondToRequest_withResult(request, CBATTError::Success);
                    }
                }
                // SAFETY: the request came from this manager.
                None => unsafe {
                    manager.respondToRequest_withResult(request, CBATTError::InvalidOffset)
                },
            }
        }

        #[unsafe(method(peripheralManager:didReceiveWriteRequests:))]
        fn did_receive_writes(
            &self,
            manager: &CBPeripheralManager,
            requests: &NSArray<CBATTRequest>,
        ) {
            if let Some(first) = requests.iter().next() {
                // SAFETY: the request came from this manager.
                unsafe {
                    manager.respondToRequest_withResult(&first, CBATTError::WriteNotPermitted)
                };
            }
        }

        #[unsafe(method(peripheralManager:didOpenL2CAPChannel:error:))]
        fn did_open_channel(
            &self,
            _manager: &CBPeripheralManager,
            channel: Option<&CBL2CAPChannel>,
            error: Option<&NSError>,
        ) {
            let r = match (channel, error) {
                (Some(c), None) => {
                    // SAFETY: no preconditions.
                    let central = unsafe { c.peer() }
                        // SAFETY: no preconditions.
                        .map(|p| {
                            unsafe { p.identifier().UUIDString() }
                                .to_string()
                                .to_lowercase()
                        })
                        .unwrap_or_default();
                    L2capStream::spawn(Shared(c.retain()))
                        .map(|stream| Session { stream, central })
                        .map_err(|e| e.to_string())
                }
                (_, e) => Err(error_text(e)),
            };
            self.send(Event::Channel(r));
        }
    }
);

impl Delegate {
    fn new(events: mpsc::Sender<Event>, name: String) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars {
            events: Mutex::new(events),
            name,
            psm: Mutex::new(None),
        });
        // SAFETY: NSObject's init has no preconditions.
        unsafe { msg_send![super(this), init] }
    }

    fn send(&self, event: Event) {
        let _ = self.ivars().events.lock().unwrap().send(event);
    }
}

fn characteristic(
    uuid: &uuid::Uuid,
    properties: CBCharacteristicProperties,
    permissions: CBAttributePermissions,
) -> Retained<CBCharacteristic> {
    // SAFETY: a dynamic value (None) is answered through the delegate.
    let c = unsafe {
        CBMutableCharacteristic::initWithType_properties_value_permissions(
            CBMutableCharacteristic::alloc(),
            &cb_uuid(uuid),
            properties,
            None,
            permissions,
        )
    };
    Retained::into_super(c)
}

/// The server's CoreBluetooth peripheral manager: advertising, the GATT
/// service and the published L2CAP channel.
pub struct Peripheral {
    _manager: Shared<CBPeripheralManager>,
    _delegate: Shared<Delegate>,
    _service: Shared<CBMutableService>,
    events: mpsc::Receiver<Event>,
    _queue: DispatchRetained<DispatchQueue>,
    pub psm: u16,
}

impl Peripheral {
    /// Publishes the channel, registers the service and starts advertising
    /// as `name`.
    pub fn start(name: &str, timeout: Duration) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let delegate = Delegate::new(tx, name.to_owned());
        let queue = DispatchQueue::new("blessh.peripheral", DispatchQueueAttr::SERIAL);
        // SAFETY: the delegate outlives the manager (both live in this struct).
        let manager = unsafe {
            CBPeripheralManager::initWithDelegate_queue(
                CBPeripheralManager::alloc(),
                Some(ProtocolObject::from_ref(&*delegate)),
                Some(&queue),
            )
        };
        let deadline = Instant::now() + timeout;
        let wait = |what: &str,
                    pick: &mut dyn FnMut(Event) -> Option<io::Result<()>>|
         -> io::Result<()> {
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                match rx.recv_timeout(left) {
                    Ok(Event::State(s)) => match s {
                        CBManagerState::PoweredOn => {
                            if let Some(r) = pick(Event::State(s)) {
                                return r;
                            }
                        }
                        CBManagerState::PoweredOff => return Err(io::Error::other("Bluetooth is off")),
                        CBManagerState::Unauthorized => {
                            return Err(io::Error::other(
                                "Bluetooth access is not allowed for this program (System Settings, Privacy & Security, Bluetooth)",
                            ))
                        }
                        CBManagerState::Unsupported => return Err(io::Error::other("this machine has no Bluetooth LE")),
                        _ => {}
                    },
                    Ok(ev) => {
                        if let Some(r) = pick(ev) {
                            return r;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        let why = if what == "waiting for Bluetooth" {
                            super::not_powered_on()
                        } else {
                            format!("{what} timed out")
                        };
                        return Err(io::Error::new(io::ErrorKind::TimedOut, why));
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => return Err(io::Error::other("Bluetooth manager gone")),
                }
            }
        };
        wait("waiting for Bluetooth", &mut |ev| {
            matches!(ev, Event::State(_)).then_some(Ok(()))
        })?;

        // SAFETY: no preconditions.
        unsafe { manager.publishL2CAPChannelWithEncryption(false) };
        let mut psm = 0u16;
        wait("publishing the L2CAP channel", &mut |ev| match ev {
            Event::Published(Ok(p)) => {
                psm = p;
                Some(Ok(()))
            }
            Event::Published(Err(e)) => Some(Err(io::Error::other(format!(
                "cannot publish an L2CAP channel: {e}"
            )))),
            _ => None,
        })?;

        // SAFETY: no preconditions.
        let service = unsafe {
            CBMutableService::initWithType_primary(
                CBMutableService::alloc(),
                &cb_uuid(&SERVICE_UUID),
                true,
            )
        };
        let chars = NSArray::from_retained_slice(&[
            characteristic(
                &INFO_UUID,
                CBCharacteristicProperties::Read,
                CBAttributePermissions::Readable,
            ),
            characteristic(
                &PSM_UUID,
                CBCharacteristicProperties::Read,
                CBAttributePermissions::Readable,
            ),
            characteristic(
                &RX_UUID,
                CBCharacteristicProperties::Write
                    | CBCharacteristicProperties::WriteWithoutResponse,
                CBAttributePermissions::Writeable,
            ),
            characteristic(
                &TX_UUID,
                CBCharacteristicProperties::Notify,
                CBAttributePermissions::Readable,
            ),
        ]);
        // SAFETY: no preconditions.
        unsafe {
            service.setCharacteristics(Some(&chars));
            manager.addService(&service);
        }
        wait("registering the service", &mut |ev| match ev {
            Event::ServiceAdded(r) => {
                Some(r.map_err(|e| io::Error::other(format!("cannot register the service: {e}"))))
            }
            _ => None,
        })?;

        let uuids = NSArray::from_retained_slice(&[cb_uuid(&SERVICE_UUID)]);
        let local_name = NSString::from_str(name);
        // SAFETY: CoreBluetooth constants.
        let (k_uuids, k_name) = unsafe {
            (
                CBAdvertisementDataServiceUUIDsKey,
                CBAdvertisementDataLocalNameKey,
            )
        };
        let uuids_obj: &AnyObject = &uuids;
        let name_obj: &AnyObject = &local_name;
        let data = NSDictionary::<NSString, AnyObject>::from_slices(
            &[k_uuids, k_name],
            &[uuids_obj, name_obj],
        );
        // SAFETY: no preconditions.
        unsafe { manager.startAdvertising(Some(&data)) };
        wait("starting to advertise", &mut |ev| match ev {
            Event::Advertising(r) => {
                Some(r.map_err(|e| io::Error::other(format!("cannot advertise: {e}"))))
            }
            _ => None,
        })?;

        Ok(Self {
            _manager: Shared(manager),
            _delegate: Shared(delegate),
            _service: Shared(service),
            events: rx,
            _queue: queue,
            psm,
        })
    }

    /// Blocks until a central opens a channel.
    pub fn next_session(&self) -> io::Result<Session> {
        loop {
            match self.events.recv() {
                Ok(Event::Channel(r)) => return r.map_err(io::Error::other),
                Ok(Event::State(s)) if s != CBManagerState::PoweredOn => {
                    return Err(io::Error::other("Bluetooth went away"))
                }
                Ok(_) => {}
                Err(_) => return Err(io::Error::other("Bluetooth manager gone")),
            }
        }
    }
}
