use std::collections::BTreeMap;
use std::sync::mpsc;
use std::thread;

use crate::model::identify;
use crate::names::consumer_name;
use crate::present::{edge_line, format_status, CollectionView, DeviceView};
use crate::report::{parse_report, InputReport};
use crate::session::{Options, Session};

#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

#[repr(C)]
struct InterfaceData {
    cb_size: u32,
    guid: Guid,
    flags: u32,
    reserved: usize,
}

#[repr(C)]
struct Attributes {
    size: u32,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
#[allow(dead_code)]
struct Caps {
    usage: u16,
    usage_page: u16,
    input: u16,
    output: u16,
    feature: u16,
    reserved: [u16; 17],
    links: u16,
    in_btn: u16,
    in_val: u16,
    in_idx: u16,
    out_btn: u16,
    out_val: u16,
    out_idx: u16,
    feat_btn: u16,
    feat_val: u16,
    feat_idx: u16,
}

const HID_GUID: Guid = Guid {
    data1: 0x4D1E_55B2,
    data2: 0xF16F,
    data3: 0x11CF,
    data4: [0x88, 0xCB, 0x00, 0x11, 0x11, 0x00, 0x00, 0x30],
};

const DIGCF_PRESENT: u32 = 0x2;
const DIGCF_DEVICEINTERFACE: u32 = 0x10;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ_WRITE: u32 = 3;
const OPEN_EXISTING: u32 = 3;

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
    fn ReadFile(
        handle: isize,
        buffer: *mut u8,
        len: u32,
        read: *mut u32,
        overlapped: *mut u8,
    ) -> i32;
    fn GetLastError() -> u32;
}

#[link(name = "hid")]
extern "system" {
    fn HidD_GetAttributes(handle: isize, attributes: *mut Attributes) -> i32;
    fn HidD_GetPreparsedData(handle: isize, data: *mut isize) -> i32;
    fn HidD_FreePreparsedData(data: isize) -> i32;
    fn HidD_GetProductString(handle: isize, buffer: *mut u16, len: u32) -> i32;
    fn HidD_GetManufacturerString(handle: isize, buffer: *mut u16, len: u32) -> i32;
    fn HidD_GetSerialNumberString(handle: isize, buffer: *mut u16, len: u32) -> i32;
    fn HidD_GetInputReport(handle: isize, buffer: *mut u8, len: u32) -> i32;
    fn HidP_GetCaps(preparsed: isize, caps: *mut Caps) -> i32;
}

#[link(name = "setupapi")]
extern "system" {
    fn SetupDiGetClassDevsW(
        class: *const Guid,
        enumerator: *const u16,
        hwnd: isize,
        flags: u32,
    ) -> isize;
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
        device: *const u8,
    ) -> i32;
    fn SetupDiDestroyDeviceInfoList(set: isize) -> i32;
}

struct Handle(isize);

impl Drop for Handle {
    fn drop(&mut self) {
        if self.0 != 0 && self.0 != -1 {
            unsafe { CloseHandle(self.0) };
        }
    }
}

struct Preparsed(isize);

impl Drop for Preparsed {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe { HidD_FreePreparsedData(self.0) };
        }
    }
}

struct Node {
    path: String,
    vendor: u16,
    product: u16,
    serial: String,
    manufacturer: String,
    product_name: String,
    usage_page: u16,
    usage: u16,
    input_len: u16,
    report: Option<InputReport>,
}

pub fn status() -> Result<String, String> {
    let devices = devices()?;
    if devices.is_empty() {
        return Err("no Magic Keyboard found".to_string());
    }
    Ok(format_status(&devices))
}

pub fn listen(options: &Options) -> Result<(), String> {
    let nodes = enumerate()?;
    if nodes.is_empty() {
        return Err("no Magic Keyboard found".to_string());
    }
    let (tx, rx) = mpsc::channel::<String>();
    let mut readers = 0usize;
    for node in nodes {
        let Some(kind) = collection_kind(node.usage_page, node.usage) else {
            continue;
        };
        if node.input_len == 0 {
            continue;
        }
        let Some(model) = identify(node.vendor, node.product) else {
            continue;
        };
        readers += 1;
        let tx = tx.clone();
        let options = *options;
        let path = node.path;
        let len = node.input_len;
        let map = model.map;
        let iso_quirk = model.iso_quirk;
        thread::spawn(move || read_loop(path, len, tx, kind, map, iso_quirk, options));
    }
    drop(tx);
    if readers == 0 {
        return Err("no readable HID collection".to_string());
    }
    println!("listening, Ctrl+C to stop");
    for line in rx {
        println!("{line}");
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Kind {
    Keyboard,
    Consumer,
    System,
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Keyboard => "keyboard",
        Kind::Consumer => "consumer",
        Kind::System => "system",
    }
}

fn kind_is_keyboard(kind: Kind) -> bool {
    matches!(kind, Kind::Keyboard)
}

fn collection_kind(usage_page: u16, usage: u16) -> Option<Kind> {
    match (usage_page, usage) {
        (0x0001, 0x0006) => Some(Kind::Keyboard),
        (0x000C, 0x0001) => Some(Kind::Consumer),
        (0x0001, 0x0080) => Some(Kind::System),
        _ => None,
    }
}

fn read_loop(
    path: String,
    input_len: u16,
    tx: mpsc::Sender<String>,
    kind: Kind,
    map: crate::model::MapKind,
    iso_quirk: bool,
    options: Options,
) {
    let handle = match open_hid(&path) {
        Ok(handle) => handle,
        Err(err) => {
            let _ = tx.send(format!("open failed ({err})"));
            return;
        }
    };
    // Country stays unknown: the inbox keyboard driver owns the boot collection.
    // Auto ISO swap needs country 13, so grave is left alone unless --iso is set.
    let mut session = Session::new(map, iso_quirk, None, options);
    let mut buffer = vec![0u8; usize::from(input_len)];
    loop {
        let mut got = 0u32;
        let ok = unsafe {
            ReadFile(
                handle.0,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                &mut got,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || got == 0 {
            let err = unsafe { GetLastError() };
            let name = kind_name(kind);
            if kind_is_keyboard(kind) && err == 5 {
                let _ = tx.send(
                    "keyboard reports are held by the inbox keyboard driver, so Fn is not visible on that collection"
                        .to_string(),
                );
            } else {
                let _ = tx.send(format!("{name} read ended ({err})"));
            }
            return;
        }
        let bytes = &buffer[..got as usize];
        match kind {
            Kind::Keyboard => {
                if let Some(InputReport::Keyboard(report)) = parse_report(bytes) {
                    for edge in session.push_keyboard(&report) {
                        let _ = tx.send(edge_line(&edge.action, edge.down));
                    }
                }
            }
            Kind::Consumer => {
                if let Some(InputReport::Consumer(report)) = parse_report(bytes) {
                    let _ = tx.send(consumer_name(report.usage));
                }
            }
            Kind::System => {
                if let Some(InputReport::System(report)) = parse_report(bytes) {
                    let dir = if report.sleep { "down" } else { "up" };
                    let _ = tx.send(format!("{dir} sleep"));
                }
            }
        }
    }
}

fn devices() -> Result<Vec<DeviceView>, String> {
    let nodes = enumerate()?;
    let mut grouped: BTreeMap<(u16, u16, String), DeviceView> = BTreeMap::new();
    for node in nodes {
        let key = (node.vendor, node.product, node.serial.clone());
        let entry = grouped.entry(key).or_insert_with(|| DeviceView {
            vendor: node.vendor,
            product: node.product,
            serial: node.serial.clone(),
            manufacturer: node.manufacturer.clone(),
            product_name: node.product_name.clone(),
            collections: Vec::new(),
        });
        if entry.product_name.is_empty() {
            entry.product_name = node.product_name;
        }
        if entry.manufacturer.is_empty() {
            entry.manufacturer = node.manufacturer;
        }
        entry.collections.push(CollectionView {
            usage_page: node.usage_page,
            usage: node.usage,
            input_len: node.input_len,
            report: node.report,
        });
    }
    let mut devices: Vec<DeviceView> = grouped.into_values().collect();
    for device in &mut devices {
        device
            .collections
            .sort_by_key(|collection| (collection.usage_page, collection.usage));
    }
    Ok(devices)
}

fn enumerate() -> Result<Vec<Node>, String> {
    let mut guid = HID_GUID;
    let set = unsafe {
        SetupDiGetClassDevsW(
            &mut guid,
            std::ptr::null(),
            0,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    };
    if set == -1 || set == 0 {
        return Err(format!("SetupDiGetClassDevsW failed ({})", unsafe {
            GetLastError()
        }));
    }
    let _guard = DeviceSet(set);
    let mut nodes = Vec::new();
    let mut index = 0u32;
    loop {
        let mut iface = InterfaceData {
            cb_size: std::mem::size_of::<InterfaceData>() as u32,
            guid: HID_GUID,
            flags: 0,
            reserved: 0,
        };
        let ok = unsafe {
            SetupDiEnumDeviceInterfaces(set, std::ptr::null(), &HID_GUID, index, &mut iface)
        };
        if ok == 0 {
            break;
        }
        index += 1;
        let Some(path) = interface_path(set, &mut iface) else {
            continue;
        };
        let Ok(handle) = open_hid(&path) else {
            continue;
        };
        let Some(node) = describe(&handle, path) else {
            continue;
        };
        if identify(node.vendor, node.product).is_some() {
            nodes.push(node);
        }
    }
    Ok(nodes)
}

struct DeviceSet(isize);

impl Drop for DeviceSet {
    fn drop(&mut self) {
        unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

fn interface_path(set: isize, iface: &mut InterfaceData) -> Option<String> {
    let mut required = 0u32;
    unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            set,
            iface,
            std::ptr::null_mut(),
            0,
            &mut required,
            std::ptr::null(),
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
            std::ptr::null(),
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

fn open_hid(path: &str) -> Result<Handle, u32> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut last = 0u32;
    for access in [GENERIC_READ | GENERIC_WRITE, GENERIC_READ, 0] {
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                access,
                FILE_SHARE_READ_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                0,
            )
        };
        if handle != -1 && handle != 0 {
            return Ok(Handle(handle));
        }
        last = unsafe { GetLastError() };
    }
    Err(last)
}

fn describe(handle: &Handle, path: String) -> Option<Node> {
    let mut attributes = Attributes {
        size: std::mem::size_of::<Attributes>() as u32,
        vendor: 0,
        product: 0,
        version: 0,
    };
    if unsafe { HidD_GetAttributes(handle.0, &mut attributes) } == 0 {
        return None;
    }
    let mut preparsed = 0isize;
    if unsafe { HidD_GetPreparsedData(handle.0, &mut preparsed) } == 0 {
        return None;
    }
    let preparsed = Preparsed(preparsed);
    let mut caps = Caps {
        usage: 0,
        usage_page: 0,
        input: 0,
        output: 0,
        feature: 0,
        reserved: [0; 17],
        links: 0,
        in_btn: 0,
        in_val: 0,
        in_idx: 0,
        out_btn: 0,
        out_val: 0,
        out_idx: 0,
        feat_btn: 0,
        feat_val: 0,
        feat_idx: 0,
    };
    let status = unsafe { HidP_GetCaps(preparsed.0, &mut caps) };
    if status < 0 {
        return None;
    }
    let report_id = report_id(caps.usage_page, caps.usage);
    let report = report_id.and_then(|id| {
        read_input(handle, id, caps.input)
            .as_deref()
            .and_then(parse_report)
    });
    Some(Node {
        path,
        vendor: attributes.vendor,
        product: attributes.product,
        serial: hid_text(handle, HidD_GetSerialNumberString),
        manufacturer: hid_text(handle, HidD_GetManufacturerString),
        product_name: hid_text(handle, HidD_GetProductString),
        usage_page: caps.usage_page,
        usage: caps.usage,
        input_len: caps.input,
        report,
    })
}

fn report_id(usage_page: u16, usage: u16) -> Option<u8> {
    match (usage_page, usage) {
        (0x0001, 0x0006) => Some(1),
        (0x000C, 0x0001) => Some(3),
        (0x0001, 0x0080) => Some(4),
        _ => None,
    }
}

fn read_input(handle: &Handle, id: u8, len: u16) -> Option<Vec<u8>> {
    if len == 0 || len > 64 {
        return None;
    }
    let mut buffer = vec![0u8; usize::from(len)];
    buffer[0] = id;
    let ok = unsafe { HidD_GetInputReport(handle.0, buffer.as_mut_ptr(), u32::from(len)) };
    if ok == 0 {
        None
    } else {
        Some(buffer)
    }
}

fn hid_text(
    handle: &Handle,
    read: unsafe extern "system" fn(isize, *mut u16, u32) -> i32,
) -> String {
    let mut buffer = [0u16; 256];
    let ok = unsafe { read(handle.0, buffer.as_mut_ptr(), (buffer.len() * 2) as u32) };
    if ok == 0 {
        return String::new();
    }
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}
