//! Read interface 1 through WinUSB and inject translated edges.
//! Held keys repeat here: Windows typematic does not run for SendInput.
//!
//! Enumeration only looks at the service name. `drive` returns before
//! CreateFile, and before any injected key, when that service is not WinUSB.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::inject::{event_fields, injection, Injection, Stroke};
use crate::model::{identify, MapKind};
use crate::present::{edge_line, step_line, Step, Translator};
use crate::report::parse_report;
use crate::session::{Action, Options};

const DIGCF_PRESENT: u32 = 0x2;
const DIGCF_ALLCLASSES: u32 = 0x4;
const DIGCF_DEVICEINTERFACE: u32 = 0x10;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ_WRITE: u32 = 3;
const OPEN_EXISTING: u32 = 3;
const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
const SPDRP_SERVICE: u32 = 4;
const KEY_READ: u32 = 0x20019;
const HKEY_LOCAL_MACHINE: isize = 0x80000002u32 as i32 as isize;
const PIPE_TRANSFER_TIMEOUT: u32 = 0x03;
const USBD_PIPE_INTERRUPT: u32 = 3;
const ERROR_SEM_TIMEOUT: u32 = 121;
const WAIT_TIMEOUT: u32 = 258;
const IDLE_POLL_MS: u32 = 200;
const REPEAT_POLL_MS: u32 = 10;
const SPI_GETKEYBOARDSPEED: u32 = 0x000A;
const SPI_GETKEYBOARDDELAY: u32 = 0x0016;
const INPUT_KEYBOARD: u32 = 1;

static STOP: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
#[repr(C)]
#[allow(dead_code)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

#[repr(C)]
#[allow(dead_code)]
struct DevInfoData {
    cb_size: u32,
    class_guid: Guid,
    dev_inst: u32,
    reserved: usize,
}

#[repr(C)]
#[allow(dead_code)]
struct InterfaceData {
    cb_size: u32,
    guid: Guid,
    flags: u32,
    reserved: usize,
}

#[repr(C)]
#[allow(dead_code)]
struct InterfaceDescriptor {
    length: u8,
    descriptor_type: u8,
    interface_number: u8,
    alternate: u8,
    num_endpoints: u8,
    class: u8,
    subclass: u8,
    protocol: u8,
    interface_index: u8,
}

#[repr(C)]
#[allow(dead_code)]
struct PipeInfo {
    pipe_type: u32,
    pipe_id: u8,
    _pad0: u8,
    maximum_packet: u16,
    interval: u8,
    _pad1: [u8; 3],
}

/// x64 `INPUT`: type, pad, then a 32-byte union. The keyboard fields sit at
/// the start of that union, with `extra` at offset 16 inside it.
#[repr(C)]
#[allow(dead_code)]
struct Input {
    kind: u32,
    _pad: u32,
    vk: u16,
    scan: u16,
    flags: u32,
    time: u32,
    _align: u32,
    extra: u64,
    _tail: [u8; 8],
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<DevInfoData>() == 32);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::offset_of!(DevInfoData, dev_inst) == 20);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::offset_of!(DevInfoData, reserved) == 24);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<PipeInfo>() == 12);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::offset_of!(PipeInfo, maximum_packet) == 6);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<InterfaceDescriptor>() == 9);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<Input>() == 40);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::offset_of!(Input, vk) == 8);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::offset_of!(Input, extra) == 24);

#[derive(Clone, Debug, PartialEq, Eq)]
struct UsbNode {
    instance: String,
    service: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pipe {
    kind: u32,
    id: u8,
    packet: u16,
}

struct Opened {
    product: u16,
    map: MapKind,
    iso_quirk: bool,
    usb: WinUsb,
    handle: Handle,
    pipe_id: u8,
    packet: u16,
}

struct Handle(isize);

impl Drop for Handle {
    fn drop(&mut self) {
        if self.0 != 0 && self.0 != -1 {
            unsafe { CloseHandle(self.0) };
        }
    }
}

struct WinUsb(*mut std::ffi::c_void);

impl Drop for WinUsb {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinUsb_Free(self.0) };
        }
    }
}

struct DeviceSet(isize);

impl Drop for DeviceSet {
    fn drop(&mut self) {
        if self.0 != 0 && self.0 != -1 {
            unsafe { SetupDiDestroyDeviceInfoList(self.0) };
        }
    }
}

struct RegKey(isize);

impl Drop for RegKey {
    fn drop(&mut self) {
        if self.0 != 0 && self.0 != -1 {
            unsafe { RegCloseKey(self.0) };
        }
    }
}

struct CtrlGuard;

impl Drop for CtrlGuard {
    fn drop(&mut self) {
        unsafe { SetConsoleCtrlHandler(Some(on_ctrl), 0) };
    }
}

struct Held {
    product: u16,
    keys: Vec<(Action, Stroke)>,
}

impl Held {
    fn remove_one(&mut self, stroke: &Stroke) {
        if let Some(index) = self
            .keys
            .iter()
            .position(|(_, held)| same_stroke(held, stroke))
        {
            self.keys.remove(index);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Typematic {
    delay_ms: u64,
    interval_ms: u64,
}

struct RepeatKey {
    stroke: Stroke,
    started: Instant,
    sent: u32,
}

struct Repeater {
    rate: Typematic,
    keys: Vec<RepeatKey>,
}

impl Repeater {
    fn new(rate: Typematic) -> Self {
        Self {
            rate,
            keys: Vec::new(),
        }
    }

    fn arm(&mut self, action: Action, stroke: Stroke) {
        if !typematic_action(action)
            || self
                .keys
                .iter()
                .any(|key| same_stroke(&key.stroke, &stroke))
        {
            return;
        }
        self.keys.push(RepeatKey {
            stroke,
            started: Instant::now(),
            sent: 0,
        });
    }

    fn disarm(&mut self, stroke: &Stroke) {
        self.keys.retain(|key| !same_stroke(&key.stroke, stroke));
    }

    fn armed(&self) -> bool {
        !self.keys.is_empty()
    }

    fn pump(&mut self, now: Instant) -> Vec<Stroke> {
        let mut out = Vec::new();
        for key in &mut self.keys {
            let elapsed = now.duration_since(key.started).as_millis() as u64;
            let Some(sent) = advance_repeat(elapsed, key.sent, self.rate) else {
                continue;
            };
            // A late wake emits one repeat and drops the missed ones.
            key.sent = sent;
            out.push(key.stroke);
        }
        out
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        for (action, stroke) in self.keys.drain(..) {
            let up = Stroke {
                down: false,
                ..stroke
            };
            if send_stroke(&up).is_ok() {
                println!("{:04x} {}", self.product, edge_line(&action, false));
            }
        }
    }
}

pub fn drive(options: &Options) -> Result<(), String> {
    if cfg!(not(target_pointer_width = "64")) {
        return Err("drive injects with the 64-bit INPUT layout".to_string());
    }
    let opened = open_winusb_keyboard()?;
    read_loop(opened, options)
}

fn open_winusb_keyboard() -> Result<Opened, String> {
    let set = usb_set()?;
    let nodes = list_interfaces(set.0)?;
    let selected = select_winusb(&nodes)?.clone();
    let (vendor, product) = parse_mi01(&selected.instance)
        .ok_or_else(|| "no Magic Keyboard interface 1 found".to_string())?;
    let model = identify(vendor, product)
        .ok_or_else(|| "no Magic Keyboard interface 1 found".to_string())?;
    let path = device_path(&selected.instance)?;
    let handle = open_device(&path)?;
    let usb = start_winusb(&handle)?;
    let pipe = interrupt_pipe(&usb)?;
    set_timeout(&usb, pipe.id, IDLE_POLL_MS)?;
    Ok(Opened {
        product: model.product,
        map: model.map,
        iso_quirk: model.iso_quirk,
        usb,
        handle,
        pipe_id: pipe.id,
        packet: pipe.packet,
    })
}

fn read_loop(opened: Opened, options: &Options) -> Result<(), String> {
    let installed = unsafe { SetConsoleCtrlHandler(Some(on_ctrl), 1) };
    if installed == 0 {
        return Err(format!("SetConsoleCtrlHandler failed ({})", last_error()));
    }
    let _ctrl = CtrlGuard;
    STOP.store(false, Ordering::SeqCst);
    let mut held = Held {
        product: opened.product,
        keys: Vec::new(),
    };
    // WinUsb_ReadPipe needs the CreateFile handle kept open.
    let _device = &opened.handle;
    let mut translator = Translator::new(opened.map, opened.iso_quirk, *options);
    let mut repeater = Repeater::new(system_typematic());
    let mut fast_poll = false;
    println!("driving {:04x}, Ctrl+C to stop", opened.product);
    let mut buffer = vec![0u8; buffer_len(opened.packet)];
    loop {
        if STOP.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut got = 0u32;
        let ok = unsafe {
            WinUsb_ReadPipe(
                opened.usb.0,
                opened.pipe_id,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                &mut got,
                std::ptr::null_mut(),
            )
        };
        if STOP.load(Ordering::Acquire) {
            return Ok(());
        }
        if ok == 0 {
            let err = last_error();
            if err != ERROR_SEM_TIMEOUT && err != WAIT_TIMEOUT {
                return Err(format!("read ended ({err})"));
            }
        } else if got > 0 {
            if let Some(report) = parse_report(&buffer[..got as usize]) {
                for step in translator.steps(&report) {
                    apply(opened.product, &step, &mut held, &mut repeater);
                }
            }
        }
        for stroke in repeater.pump(Instant::now()) {
            let _ = send_stroke(&stroke);
        }
        let want_fast = repeater.armed();
        if want_fast != fast_poll
            && set_timeout(
                &opened.usb,
                opened.pipe_id,
                if want_fast {
                    REPEAT_POLL_MS
                } else {
                    IDLE_POLL_MS
                },
            )
            .is_ok()
        {
            fast_poll = want_fast;
        }
    }
}

fn apply(product: u16, step: &Step, held: &mut Held, repeater: &mut Repeater) {
    match step {
        Step::Edge(edge) => match injection(edge.action, edge.down) {
            Injection::Skip => {}
            Injection::Unmapped => {
                println!(
                    "{product:04x} unmapped {}",
                    edge_line(&edge.action, edge.down)
                );
            }
            Injection::Stroke(stroke) => match send_stroke(&stroke) {
                Ok(()) => {
                    println!("{product:04x} {}", edge_line(&edge.action, edge.down));
                    if stroke.down {
                        held.keys.push((edge.action, stroke));
                        repeater.arm(edge.action, stroke);
                    } else {
                        held.remove_one(&stroke);
                        repeater.disarm(&stroke);
                    }
                }
                Err(err) => eprintln!("{product:04x} inject failed ({err})"),
            },
        },
        Step::Consumer(_, _) => {
            println!("{product:04x} unmapped {}", step_line(step));
        }
    }
}

fn send_stroke(stroke: &Stroke) -> Result<(), u32> {
    let (vk, scan, flags) = event_fields(stroke);
    let input = Input {
        kind: INPUT_KEYBOARD,
        _pad: 0,
        vk,
        scan,
        flags,
        time: 0,
        _align: 0,
        extra: 0,
        _tail: [0; 8],
    };
    let sent = unsafe { SendInput(1, &input, std::mem::size_of::<Input>() as i32) };
    if sent == 1 {
        Ok(())
    } else {
        Err(last_error())
    }
}

fn parse_mi01(instance: &str) -> Option<(u16, u16)> {
    let upper = instance.to_ascii_uppercase();
    let rest = upper.strip_prefix("USB\\VID_")?;
    let (vid, rest) = rest.split_once("&PID_")?;
    let (pid, rest) = rest.split_once("&MI_")?;
    let mi = rest.split(['\\', '&']).next()?;
    if mi != "01" {
        return None;
    }
    Some((
        u16::from_str_radix(vid, 16).ok()?,
        u16::from_str_radix(pid, 16).ok()?,
    ))
}

fn select_winusb(nodes: &[UsbNode]) -> Result<&UsbNode, String> {
    let known: Vec<&UsbNode> = nodes
        .iter()
        .filter(|node| {
            parse_mi01(&node.instance)
                .is_some_and(|(vendor, product)| identify(vendor, product).is_some())
        })
        .collect();
    if known.is_empty() {
        return Err("no Magic Keyboard interface 1 found".to_string());
    }
    let winusb: Vec<&UsbNode> = known
        .iter()
        .copied()
        .filter(|node| node.service.eq_ignore_ascii_case("winusb"))
        .collect();
    match winusb.as_slice() {
        [one] => Ok(*one),
        [] => {
            if known
                .iter()
                .all(|node| node.service.is_empty() || node.service.eq_ignore_ascii_case("hidusb"))
            {
                Err("interface 1 is not on WinUSB".to_string())
            } else {
                let service = known
                    .iter()
                    .find(|node| {
                        !node.service.is_empty() && !node.service.eq_ignore_ascii_case("hidusb")
                    })
                    .map(|node| node.service.as_str())
                    .unwrap_or("unknown");
                Err(format!("interface 1 is bound to {service}, not WinUSB"))
            }
        }
        _ => Err("more than one Magic Keyboard interface 1 is on WinUSB".to_string()),
    }
}

fn first_interrupt_in(pipes: &[Pipe]) -> Option<Pipe> {
    pipes
        .iter()
        .copied()
        .find(|pipe| pipe.kind == USBD_PIPE_INTERRUPT && pipe.id & 0x80 != 0)
}

fn buffer_len(packet: u16) -> usize {
    match usize::from(packet) {
        0 => 16,
        len => len.min(1024),
    }
}

fn parse_guid(text: &str) -> Option<Guid> {
    let text = text.trim().trim_start_matches('{').trim_end_matches('}');
    let mut parts = text.split('-');
    let data1 = parts.next()?;
    let data2 = parts.next()?;
    let data3 = parts.next()?;
    let fourth = parts.next()?;
    let fifth = parts.next()?;
    if parts.next().is_some()
        || data1.len() != 8
        || data2.len() != 4
        || data3.len() != 4
        || fourth.len() != 4
        || fifth.len() != 12
    {
        return None;
    }
    let mut data4 = [0u8; 8];
    data4[0] = u8::from_str_radix(&fourth[0..2], 16).ok()?;
    data4[1] = u8::from_str_radix(&fourth[2..4], 16).ok()?;
    for index in 0..6 {
        let start = index * 2;
        data4[index + 2] = u8::from_str_radix(&fifth[start..start + 2], 16).ok()?;
    }
    Some(Guid {
        data1: u32::from_str_radix(data1, 16).ok()?,
        data2: u16::from_str_radix(data2, 16).ok()?,
        data3: u16::from_str_radix(data3, 16).ok()?,
        data4,
    })
}

fn usb_set() -> Result<DeviceSet, String> {
    let name = wide("USB");
    let set = unsafe {
        SetupDiGetClassDevsW(
            std::ptr::null(),
            name.as_ptr(),
            0,
            DIGCF_PRESENT | DIGCF_ALLCLASSES,
        )
    };
    if set == 0 || set == -1 {
        Err(format!("SetupDiGetClassDevsW failed ({})", last_error()))
    } else {
        Ok(DeviceSet(set))
    }
}

fn list_interfaces(set: isize) -> Result<Vec<UsbNode>, String> {
    let mut nodes = Vec::new();
    let mut index = 0u32;
    loop {
        let mut dev = empty_dev();
        let ok = unsafe { SetupDiEnumDeviceInfo(set, index, &mut dev) };
        if ok == 0 {
            break;
        }
        index += 1;
        let Some(instance) = device_instance_id(set, &mut dev) else {
            continue;
        };
        if parse_mi01(&instance).is_none() {
            continue;
        }
        let service = device_service(set, &mut dev).unwrap_or_default();
        nodes.push(UsbNode { instance, service });
    }
    Ok(nodes)
}

fn device_path(instance: &str) -> Result<String, String> {
    let guids = interface_guids(instance)?;
    if guids.is_empty() {
        return Err("interface 1 has no WinUSB device interface".to_string());
    }
    let mut missed = "interface 1 has no WinUSB device interface".to_string();
    for guid in &guids {
        match path_for_guid(guid, instance) {
            Ok(path) => return Ok(path),
            Err(err) => missed = err,
        }
    }
    Err(missed)
}

fn parameters_key(instance: &str) -> String {
    format!(r"SYSTEM\CurrentControlSet\Enum\{instance}\Device Parameters")
}

fn interface_guids(instance: &str) -> Result<Vec<Guid>, String> {
    // The interface GUID is on the Enum key. SetupDiOpenDevRegKey does not
    // open that key for this device.
    let name = wide(&parameters_key(instance));
    let mut params = 0isize;
    let status =
        unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, name.as_ptr(), 0, KEY_READ, &mut params) };
    if status != 0 {
        return Err(format!(
            "interface 1 has no WinUSB device interface ({status})"
        ));
    }
    let params = RegKey(params);
    let mut guids = Vec::new();
    for name in ["DeviceInterfaceGUIDs", "DeviceInterfaceGUID"] {
        for text in reg_strings(params.0, name) {
            if let Some(guid) = parse_guid(&text) {
                guids.push(guid);
            }
        }
    }
    Ok(guids)
}

fn path_for_guid(guid: &Guid, instance: &str) -> Result<String, String> {
    let set = unsafe {
        SetupDiGetClassDevsW(
            guid,
            std::ptr::null(),
            0,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    };
    if set == 0 || set == -1 {
        return Err(format!("SetupDiGetClassDevsW failed ({})", last_error()));
    }
    let _set = DeviceSet(set);
    let mut index = 0u32;
    loop {
        let mut iface = InterfaceData {
            cb_size: std::mem::size_of::<InterfaceData>() as u32,
            guid: *guid,
            flags: 0,
            reserved: 0,
        };
        let ok =
            unsafe { SetupDiEnumDeviceInterfaces(set, std::ptr::null(), guid, index, &mut iface) };
        if ok == 0 {
            break;
        }
        index += 1;
        let mut dev = empty_dev();
        let Some(path) = interface_path(set, &mut iface, &mut dev) else {
            continue;
        };
        let Some(id) = device_instance_id(set, &mut dev) else {
            continue;
        };
        if id.eq_ignore_ascii_case(instance) {
            return Ok(path);
        }
    }
    Err("interface 1 has no WinUSB device interface".to_string())
}

fn interface_path(set: isize, iface: &mut InterfaceData, dev: &mut DevInfoData) -> Option<String> {
    let mut required = 0u32;
    unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            set,
            iface,
            std::ptr::null_mut(),
            0,
            &mut required,
            std::ptr::null_mut(),
        );
    }
    if required < 8 {
        return None;
    }
    let mut detail = vec![0u8; required as usize];
    detail[..4].copy_from_slice(&8u32.to_le_bytes());
    let ok = unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            set,
            iface,
            detail.as_mut_ptr(),
            required,
            &mut required,
            dev,
        )
    };
    if ok == 0 {
        return None;
    }
    let words: Vec<u16> = detail[4..]
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    if words.is_empty() {
        None
    } else {
        Some(String::from_utf16_lossy(&words))
    }
}

fn interrupt_pipe(usb: &WinUsb) -> Result<Pipe, String> {
    let mut descriptor = InterfaceDescriptor {
        length: 0,
        descriptor_type: 0,
        interface_number: 0,
        alternate: 0,
        num_endpoints: 0,
        class: 0,
        subclass: 0,
        protocol: 0,
        interface_index: 0,
    };
    let ok = unsafe { WinUsb_QueryInterfaceSettings(usb.0, 0, &mut descriptor) };
    if ok == 0 {
        return Err(format!(
            "WinUsb_QueryInterfaceSettings failed ({})",
            last_error()
        ));
    }
    let mut pipes = Vec::with_capacity(usize::from(descriptor.num_endpoints));
    for index in 0..descriptor.num_endpoints {
        let mut info = PipeInfo {
            pipe_type: 0,
            pipe_id: 0,
            _pad0: 0,
            maximum_packet: 0,
            interval: 0,
            _pad1: [0; 3],
        };
        let ok = unsafe { WinUsb_QueryPipe(usb.0, 0, index, &mut info) };
        if ok == 0 {
            return Err(format!("WinUsb_QueryPipe failed ({})", last_error()));
        }
        pipes.push(Pipe {
            kind: info.pipe_type,
            id: info.pipe_id,
            packet: info.maximum_packet,
        });
    }
    first_interrupt_in(&pipes).ok_or_else(|| "interface 1 has no interrupt in pipe".to_string())
}

fn set_timeout(usb: &WinUsb, pipe: u8, timeout: u32) -> Result<(), String> {
    let ok = unsafe {
        WinUsb_SetPipePolicy(
            usb.0,
            pipe,
            PIPE_TRANSFER_TIMEOUT,
            4,
            &timeout as *const u32 as *const u8,
        )
    };
    if ok == 0 {
        Err(format!("could not set the pipe timeout ({})", last_error()))
    } else {
        Ok(())
    }
}

fn same_stroke(left: &Stroke, right: &Stroke) -> bool {
    left.kind == right.kind && left.code == right.code && left.extended == right.extended
}

fn typematic_action(action: Action) -> bool {
    matches!(
        action,
        Action::Key(_) | Action::VolumeDown | Action::VolumeUp
    )
}

fn typematic_from_spi(delay_index: u32, speed: u32) -> Typematic {
    // Delay indexes 0..3 are 250..1000 ms. Speed 0..31 is about 2.5..30 Hz.
    let delay_index = delay_index.min(3);
    let speed = u64::from(speed.min(31));
    let per_second_milli = 2500 + speed * 27500 / 31;
    Typematic {
        delay_ms: 250 * u64::from(delay_index + 1),
        interval_ms: (1_000_000 / per_second_milli).max(1),
    }
}

fn system_typematic() -> Typematic {
    let mut speed = 31u32;
    let mut delay = 1u32;
    unsafe {
        if SystemParametersInfoW(
            SPI_GETKEYBOARDSPEED,
            0,
            &mut speed as *mut u32 as *mut u8,
            0,
        ) == 0
        {
            speed = 31;
        }
        if SystemParametersInfoW(
            SPI_GETKEYBOARDDELAY,
            0,
            &mut delay as *mut u32 as *mut u8,
            0,
        ) == 0
        {
            delay = 1;
        }
    }
    typematic_from_spi(delay, speed)
}

fn repeats_due(elapsed_ms: u64, rate: Typematic, sent: u32) -> u32 {
    if rate.interval_ms == 0 || elapsed_ms < rate.delay_ms {
        return 0;
    }
    let past = elapsed_ms - rate.delay_ms;
    let should = 1 + past / rate.interval_ms;
    let should = u32::try_from(should).unwrap_or(u32::MAX);
    should.saturating_sub(sent)
}

fn advance_repeat(elapsed_ms: u64, sent: u32, rate: Typematic) -> Option<u32> {
    let due = repeats_due(elapsed_ms, rate, sent);
    if due == 0 {
        None
    } else {
        sent.checked_add(due)
    }
}

fn device_open_flags() -> u32 {
    // WinUsb_Initialize returns error 6 unless CreateFile requested overlapped I/O.
    FILE_FLAG_OVERLAPPED
}

fn open_device(path: &str) -> Result<Handle, String> {
    let wide_path = wide(path);
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            device_open_flags(),
            0,
        )
    };
    if handle == 0 || handle == -1 {
        Err(format!("open failed ({})", last_error()))
    } else {
        Ok(Handle(handle))
    }
}

fn start_winusb(handle: &Handle) -> Result<WinUsb, String> {
    let mut interface = std::ptr::null_mut();
    let ok = unsafe { WinUsb_Initialize(handle.0, &mut interface) };
    if ok == 0 || interface.is_null() {
        Err(format!("WinUsb_Initialize failed ({})", last_error()))
    } else {
        Ok(WinUsb(interface))
    }
}

fn device_instance_id(set: isize, dev: &mut DevInfoData) -> Option<String> {
    let mut buf = vec![0u16; 256];
    let mut required = 0u32;
    let ok = unsafe {
        SetupDiGetDeviceInstanceIdW(set, dev, buf.as_mut_ptr(), buf.len() as u32, &mut required)
    };
    if ok == 0 {
        if required <= 1 || required > 1024 {
            return None;
        }
        buf.resize(required as usize, 0);
        let ok = unsafe {
            SetupDiGetDeviceInstanceIdW(set, dev, buf.as_mut_ptr(), buf.len() as u32, &mut required)
        };
        if ok == 0 {
            return None;
        }
    }
    let text = from_wide(&buf);
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn device_service(set: isize, dev: &mut DevInfoData) -> Option<String> {
    let mut buf = [0u16; 64];
    let mut kind = 0u32;
    let mut required = 0u32;
    let ok = unsafe {
        SetupDiGetDeviceRegistryPropertyW(
            set,
            dev,
            SPDRP_SERVICE,
            &mut kind,
            buf.as_mut_ptr() as *mut u8,
            (buf.len() * 2) as u32,
            &mut required,
        )
    };
    if ok == 0 {
        None
    } else {
        Some(from_wide(&buf))
    }
}

fn reg_strings(key: isize, name: &str) -> Vec<String> {
    let wide_name = wide(name);
    let mut kind = 0u32;
    let mut len = 0u32;
    let status = unsafe {
        RegQueryValueExW(
            key,
            wide_name.as_ptr(),
            std::ptr::null_mut(),
            &mut kind,
            std::ptr::null_mut(),
            &mut len,
        )
    };
    if (status != 0 && status != 234) || len == 0 || len > 4096 {
        return Vec::new();
    }
    let mut buf = vec![0u8; len as usize];
    let mut got = len;
    let status = unsafe {
        RegQueryValueExW(
            key,
            wide_name.as_ptr(),
            std::ptr::null_mut(),
            &mut kind,
            buf.as_mut_ptr(),
            &mut got,
        )
    };
    if status != 0 || (kind != 1 && kind != 7) {
        return Vec::new();
    }
    buf.truncate(got as usize);
    from_multi(&buf)
}

fn from_multi(bytes: &[u8]) -> Vec<String> {
    let words: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    let mut out = Vec::new();
    let mut start = 0usize;
    for (index, unit) in words.iter().enumerate() {
        if *unit == 0 {
            if index > start {
                out.push(String::from_utf16_lossy(&words[start..index]));
            }
            start = index + 1;
        }
    }
    out
}

fn empty_dev() -> DevInfoData {
    DevInfoData {
        cb_size: std::mem::size_of::<DevInfoData>() as u32,
        class_guid: Guid {
            data1: 0,
            data2: 0,
            data3: 0,
            data4: [0; 8],
        },
        dev_inst: 0,
        reserved: 0,
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|unit| *unit == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

fn last_error() -> u32 {
    unsafe { GetLastError() }
}

unsafe extern "system" fn on_ctrl(code: u32) -> i32 {
    if code == 0 || code == 1 {
        STOP.store(true, Ordering::SeqCst);
        1
    } else {
        0
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *const u8,
        disposition: u32,
        flags: u32,
        template: isize,
    ) -> isize;
    fn CloseHandle(handle: isize) -> i32;
    fn GetLastError() -> u32;
    fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
}

// winhid declares the same DLL entries with its own Guid type. The ABI matches.
#[allow(clashing_extern_declarations)]
#[link(name = "setupapi")]
extern "system" {
    fn SetupDiGetClassDevsW(
        class: *const Guid,
        enumerator: *const u16,
        hwnd: isize,
        flags: u32,
    ) -> isize;
    fn SetupDiEnumDeviceInfo(set: isize, index: u32, data: *mut DevInfoData) -> i32;
    fn SetupDiDestroyDeviceInfoList(set: isize) -> i32;
    fn SetupDiGetDeviceInstanceIdW(
        set: isize,
        data: *mut DevInfoData,
        id: *mut u16,
        size: u32,
        required: *mut u32,
    ) -> i32;
    fn SetupDiGetDeviceRegistryPropertyW(
        set: isize,
        data: *mut DevInfoData,
        property: u32,
        reg_type: *mut u32,
        buffer: *mut u8,
        size: u32,
        required: *mut u32,
    ) -> i32;
    fn SetupDiEnumDeviceInterfaces(
        set: isize,
        device: *const u8,
        class: *const Guid,
        index: u32,
        data: *mut InterfaceData,
    ) -> i32;
    fn SetupDiGetDeviceInterfaceDetailW(
        set: isize,
        data: *mut InterfaceData,
        detail: *mut u8,
        size: u32,
        required: *mut u32,
        device: *mut DevInfoData,
    ) -> i32;
}

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(key: isize, sub: *const u16, options: u32, sam: u32, out: *mut isize) -> i32;
    fn RegQueryValueExW(
        key: isize,
        name: *const u16,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        len: *mut u32,
    ) -> i32;
    fn RegCloseKey(key: isize) -> i32;
}

#[link(name = "winusb")]
extern "system" {
    fn WinUsb_Initialize(device: isize, interface: *mut *mut std::ffi::c_void) -> i32;
    fn WinUsb_Free(interface: *mut std::ffi::c_void) -> i32;
    fn WinUsb_QueryInterfaceSettings(
        interface: *mut std::ffi::c_void,
        alternate: u8,
        descriptor: *mut InterfaceDescriptor,
    ) -> i32;
    fn WinUsb_QueryPipe(
        interface: *mut std::ffi::c_void,
        alternate: u8,
        index: u8,
        info: *mut PipeInfo,
    ) -> i32;
    fn WinUsb_SetPipePolicy(
        interface: *mut std::ffi::c_void,
        pipe: u8,
        policy: u32,
        len: u32,
        value: *const u8,
    ) -> i32;
    fn WinUsb_ReadPipe(
        interface: *mut std::ffi::c_void,
        pipe: u8,
        buffer: *mut u8,
        len: u32,
        transferred: *mut u32,
        overlapped: *mut u8,
    ) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn SendInput(count: u32, inputs: *const Input, size: i32) -> u32;
    fn SystemParametersInfoW(action: u32, param: u32, value: *mut u8, winini: u32) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(product: &str, service: &str) -> UsbNode {
        UsbNode {
            instance: format!("USB\\VID_05AC&PID_{product}&MI_01\\INSTANCE"),
            service: service.to_string(),
        }
    }

    #[test]
    fn hidusb_is_not_opened() {
        let nodes = [node("0321", "HidUsb")];
        assert_eq!(
            select_winusb(&nodes).unwrap_err(),
            "interface 1 is not on WinUSB"
        );
    }

    #[test]
    fn a_missing_interface_names_the_gap() {
        assert_eq!(
            select_winusb(&[]).unwrap_err(),
            "no Magic Keyboard interface 1 found"
        );
        let trackpad = UsbNode {
            instance: "USB\\VID_05AC&PID_0265&MI_01\\INSTANCE".to_string(),
            service: "WINUSB".to_string(),
        };
        assert_eq!(
            select_winusb(&[trackpad]).unwrap_err(),
            "no Magic Keyboard interface 1 found"
        );
    }

    #[test]
    fn winusb_selects_the_bound_interface() {
        let nodes = [node("0321", "HidUsb"), node("0267", "WINUSB")];
        let picked = select_winusb(&nodes).unwrap();
        assert!(picked.instance.contains("0267"));
        assert!(picked.service.eq_ignore_ascii_case("winusb"));
    }

    #[test]
    fn two_winusb_interfaces_are_refused() {
        let nodes = [node("0321", "WinUSB"), node("0267", "winusb")];
        assert_eq!(
            select_winusb(&nodes).unwrap_err(),
            "more than one Magic Keyboard interface 1 is on WinUSB"
        );
    }

    #[test]
    fn another_service_is_named() {
        let nodes = [node("0321", "usbccgp")];
        assert_eq!(
            select_winusb(&nodes).unwrap_err(),
            "interface 1 is bound to usbccgp, not WinUSB"
        );
    }

    #[test]
    fn mi01_parse_rejects_other_interfaces() {
        assert_eq!(
            parse_mi01(r"USB\VID_05AC&PID_0321&MI_01\INSTANCE"),
            Some((0x05AC, 0x0321))
        );
        assert_eq!(
            parse_mi01(r"usb\vid_05ac&pid_0267&mi_01"),
            Some((0x05AC, 0x0267))
        );
        assert_eq!(parse_mi01(r"USB\VID_05AC&PID_0321&MI_00\INSTANCE"), None);
        assert_eq!(parse_mi01(r"USB\VID_05AC&PID_0321\PARENT"), None);
    }

    #[test]
    fn interrupt_in_is_the_first_in_pipe() {
        let pipes = [
            Pipe {
                kind: 2,
                id: 0x01,
                packet: 64,
            },
            Pipe {
                kind: USBD_PIPE_INTERRUPT,
                id: 0x82,
                packet: 16,
            },
        ];
        let pipe = first_interrupt_in(&pipes).unwrap();
        assert_eq!(pipe.id, 0x82);
        assert_eq!(pipe.packet, 16);
        assert_eq!(buffer_len(pipe.packet), 16);
        let other = [Pipe {
            kind: USBD_PIPE_INTERRUPT,
            id: 0x81,
            packet: 8,
        }];
        assert_eq!(first_interrupt_in(&other).unwrap().id, 0x81);
        assert!(first_interrupt_in(&[Pipe {
            kind: USBD_PIPE_INTERRUPT,
            id: 0x02,
            packet: 16,
        }])
        .is_none());
    }

    #[test]
    fn winusb_open_requests_overlapped_io() {
        assert_eq!(device_open_flags(), 0x4000_0000);
    }

    #[test]
    fn parameters_live_on_the_enum_key() {
        assert_eq!(
            parameters_key(r"USB\VID_05AC&PID_0321&MI_01\INSTANCE"),
            r"SYSTEM\CurrentControlSet\Enum\USB\VID_05AC&PID_0321&MI_01\INSTANCE\Device Parameters"
        );
    }

    #[test]
    fn parses_a_device_interface_guid() {
        let guid = parse_guid("{A5DCBF10-6530-11D2-901F-00C04FB951ED}").unwrap();
        assert_eq!(guid.data1, 0xA5DC_BF10);
        assert_eq!(guid.data2, 0x6530);
        assert_eq!(guid.data3, 0x11D2);
        assert_eq!(guid.data4, [0x90, 0x1F, 0x00, 0xC0, 0x4F, 0xB9, 0x51, 0xED]);
        assert!(parse_guid("not-a-guid").is_none());
    }

    #[test]
    fn held_key_repeats_after_the_typematic_delay() {
        let rate = typematic_from_spi(1, 31);
        assert_eq!(rate.delay_ms, 500);
        assert_eq!(rate.interval_ms, 33);
        assert_eq!(advance_repeat(499, 0, rate), None);
        assert_eq!(advance_repeat(500, 0, rate), Some(1));
        assert_eq!(advance_repeat(500, 1, rate), None);
        assert_eq!(advance_repeat(532, 1, rate), None);
        assert_eq!(advance_repeat(533, 1, rate), Some(2));
        let stalled = advance_repeat(5_000, 0, rate).unwrap();
        assert!(stalled > 1);
        assert_eq!(advance_repeat(5_000, stalled, rate), None);
    }

    #[test]
    fn typematic_endpoints_match_windows() {
        assert_eq!(typematic_from_spi(0, 0).delay_ms, 250);
        assert_eq!(typematic_from_spi(0, 0).interval_ms, 400);
        assert_eq!(typematic_from_spi(3, 31).delay_ms, 1000);
        assert_eq!(typematic_from_spi(3, 31).interval_ms, 33);
        assert_eq!(typematic_from_spi(9, 100).delay_ms, 1000);
        assert_eq!(typematic_from_spi(9, 100).interval_ms, 33);
    }

    #[test]
    fn letters_repeat_and_toggles_do_not() {
        assert!(typematic_action(Action::Key(0x2A)));
        assert!(typematic_action(Action::VolumeDown));
        assert!(typematic_action(Action::VolumeUp));
        assert!(!typematic_action(Action::Sleep));
        assert!(!typematic_action(Action::Mute));
        assert!(!typematic_action(Action::NumLock));
        assert!(!typematic_action(Action::Search));
        assert!(!typematic_action(Action::PlayPause));
        assert!(!typematic_action(Action::TrackNext));
        assert!(!typematic_action(Action::TrackPrevious));
        assert!(!typematic_action(Action::Function));
    }

    #[test]
    fn send_input_struct_is_40_bytes_on_x64() {
        if cfg!(target_pointer_width = "64") {
            assert_eq!(std::mem::size_of::<Input>(), 40);
            assert_eq!(std::mem::offset_of!(Input, extra), 24);
        }
    }
}
