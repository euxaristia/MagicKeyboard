//! Raw Input view of Apple keyboards.
//!
//! The inbox keyboard driver owns the boot collection, so `ReadFile` there
//! returns access denied. Raw Input still delivers that collection as scan
//! codes. Other top-level collections arrive as HID reports when Windows
//! exposes them.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::mpsc;
use std::thread;

use crate::model::identify;
use crate::present::Translator;
use crate::report::{parse_report, InputReport};
use crate::scancode::format_scan;
use crate::session::Options;

const RIM_TYPEKEYBOARD: u32 = 1;
const RIM_TYPEHID: u32 = 2;
const RIDI_DEVICENAME: u32 = 0x20000007;
const RIDI_DEVICEINFO: u32 = 0x2000000b;
const RID_INPUT: u32 = 0x10000003;
const RIDEV_PAGEONLY: u32 = 0x00000020;
const RIDEV_INPUTSINK: u32 = 0x00000100;
const RIDEV_REMOVE: u32 = 0x00000001;
const WM_INPUT: u32 = 0x00FF;
const WM_QUIT: u32 = 0x0012;
const WM_STOP: u32 = 0x8000;
const PM_REMOVE: u32 = 0x0001;
const QS_ALLINPUT: u32 = 0x04FF;
static RAW_HWND: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

#[repr(C)]
struct DeviceEntry {
    device: *mut std::ffi::c_void,
    kind: u32,
}

#[repr(C)]
struct Header {
    kind: u32,
    size: u32,
    device: usize,
    wparam: usize,
}

#[repr(C)]
struct RawInputDevice {
    usage_page: u16,
    usage: u16,
    flags: u32,
    target: *mut std::ffi::c_void,
}

#[repr(C)]
struct DeviceInfo {
    cb_size: u32,
    kind: u32,
    data: [u8; 24],
}

#[repr(C)]
struct WndClass {
    cb_size: u32,
    style: u32,
    wnd_proc: Option<unsafe extern "system" fn(*mut std::ffi::c_void, u32, usize, isize) -> isize>,
    cls_extra: i32,
    wnd_extra: i32,
    instance: *mut std::ffi::c_void,
    icon: *mut std::ffi::c_void,
    cursor: *mut std::ffi::c_void,
    background: *mut std::ffi::c_void,
    menu: *const u16,
    class: *const u16,
    icon_sm: *mut std::ffi::c_void,
}

#[repr(C)]
struct Msg {
    hwnd: *mut std::ffi::c_void,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    x: i32,
    y: i32,
    private: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawKind {
    Mouse,
    Keyboard,
    Hid,
    Other(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawDevice {
    pub handle: usize,
    pub kind: RawKind,
    pub vendor: u16,
    pub product: u16,
    pub usage_page: u16,
    pub usage: u16,
    pub name: String,
}

pub fn parse_hid_ids(name: &str) -> Option<(u16, u16)> {
    let upper = name.to_ascii_uppercase();
    let vendor = tagged_id(&upper, &["VID_", "VID&"])?;
    let product = tagged_id(&upper, &["PID_", "PID&"])?;
    Some((vendor, product))
}

fn tagged_id(upper: &str, tags: &[&str]) -> Option<u16> {
    for tag in tags {
        let Some(pos) = upper.find(tag) else {
            continue;
        };
        let rest = &upper[pos + tag.len()..];
        let hex: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        if hex.len() < 4 {
            continue;
        }
        if let Ok(value) = u16::from_str_radix(&hex[hex.len() - 4..], 16) {
            return Some(value);
        }
    }
    None
}

pub fn apple_device(vendor: u16, product: u16) -> bool {
    vendor == 0x05AC || vendor == 0x004C || identify(vendor, product).is_some()
}

pub fn format_raw_list(apple: &[RawDevice], total: usize) -> String {
    let mut out = String::new();
    if apple.is_empty() {
        out.push_str("no Apple device in raw input\n");
    }
    for device in apple {
        let kind = match device.kind {
            RawKind::Mouse => "mouse",
            RawKind::Keyboard => "keyboard",
            RawKind::Hid => "hid",
            RawKind::Other(value) => {
                out.push_str(&format!(
                    "other {value} {:04x}:{:04x} usage {:04x}:{:04x}\n  {}\n",
                    device.vendor, device.product, device.usage_page, device.usage, device.name
                ));
                continue;
            }
        };
        let label = identify(device.vendor, device.product)
            .map(|model| model.name)
            .unwrap_or("Apple device");
        out.push_str(&format!(
            "{kind} {:04x}:{:04x} usage {:04x}:{:04x} {label}\n  {}\n",
            device.vendor, device.product, device.usage_page, device.usage, device.name
        ));
    }
    out.push_str(&format!("{total} raw devices\n"));
    out
}

fn format_keyboard(product: u16, data: &[u8]) -> Option<String> {
    if data.len() < 8 {
        return None;
    }
    let make = u16::from_le_bytes([data[0], data[1]]);
    let flags = u16::from_le_bytes([data[2], data[3]]);
    Some(format_scan(product, make, flags))
}

fn format_hid(product: u16, usage_page: u16, usage: u16, data: &[u8]) -> String {
    if data.len() < 8 {
        return format!("hid product {product:04x} usage {usage_page:04x}:{usage:04x} short");
    }
    let size = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
    let count = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
    let take = size.saturating_mul(count).min(data.len().saturating_sub(8));
    let bytes = &data[8..8 + take];
    let mut line = format!(
        "hid product {product:04x} usage {usage_page:04x}:{usage:04x} {}",
        hex_bytes(bytes)
    );
    if let Some(parsed) = parse_report(bytes) {
        line.push(' ');
        line.push_str(&report_note(&parsed));
    }
    line
}

fn report_note(report: &InputReport) -> String {
    match report {
        InputReport::Keyboard(report) => {
            let function = if report.function { "down" } else { "up" };
            format!("fn {function} modifiers {:02x}", report.modifiers)
        }
        InputReport::Consumer(report) => format!("consumer {:04x}", report.usage),
        InputReport::System(report) => {
            let sleep = if report.sleep { "down" } else { "up" };
            format!("sleep {sleep}")
        }
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn raw_status() -> Result<String, String> {
    let (apple, total) = apple_devices()?;
    Ok(format_raw_list(&apple, total))
}

pub fn listen_raw(seconds: Option<u64>, options: &Options) -> Result<(), String> {
    let (apple, total) = apple_devices()?;
    print!("{}", format_raw_list(&apple, total));
    let window = RawWindow::open(true)?;
    window.register()?;
    println!("raw input registered, Ctrl+C to stop");
    window.pump(&apple, seconds, options, None)?;
    Ok(())
}

/// Names keys from the boot collection while `listen` reads the other reports.
pub fn watch_scan_codes(tx: mpsc::Sender<String>) {
    thread::spawn(move || {
        let result = (|| {
            let (apple, _) = apple_devices()?;
            let window = RawWindow::open(false)?;
            window.register()?;
            window.pump(&apple, None, &Options::default(), Some(&tx))
        })();
        if let Err(message) = result {
            let _ = tx.send(message);
        }
    });
}

fn apple_devices() -> Result<(Vec<RawDevice>, usize), String> {
    let entries = device_entries()?;
    let total = entries.len();
    let mut apple = Vec::new();
    for entry in entries {
        if entry.device.is_null() {
            continue;
        }
        let Some(device) = describe(entry.device as usize, entry.device, entry.kind) else {
            continue;
        };
        if apple_device(device.vendor, device.product) {
            apple.push(device);
        }
    }
    Ok((apple, total))
}

fn device_entries() -> Result<Vec<DeviceEntry>, String> {
    let size = std::mem::size_of::<DeviceEntry>() as u32;
    let mut count = 0u32;
    let rc = unsafe { GetRawInputDeviceList(std::ptr::null_mut(), &mut count, size) };
    if rc == u32::MAX {
        return Err(format!("raw device list failed ({})", last_error()));
    }
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut entries = Vec::with_capacity(count as usize);
    entries.resize_with(count as usize, || DeviceEntry {
        device: std::ptr::null_mut(),
        kind: 0,
    });
    let written = unsafe { GetRawInputDeviceList(entries.as_mut_ptr(), &mut count, size) };
    if written == u32::MAX {
        return Err(format!("raw device list failed ({})", last_error()));
    }
    entries.truncate(written as usize);
    Ok(entries)
}

fn describe(handle: usize, device: *mut std::ffi::c_void, kind: u32) -> Option<RawDevice> {
    let name = device_name(device);
    let (name_vendor, name_product) = parse_hid_ids(&name).unwrap_or((0, 0));
    let info = device_info(device);
    let (vendor, product, usage_page, usage) = match info {
        Some((RIM_TYPEHID, data)) if data.len() >= 16 => {
            let vendor = u32::from_le_bytes(data[0..4].try_into().ok()?) as u16;
            let product = u32::from_le_bytes(data[4..8].try_into().ok()?) as u16;
            let usage_page = u16::from_le_bytes(data[12..14].try_into().ok()?);
            let usage = u16::from_le_bytes(data[14..16].try_into().ok()?);
            (vendor, product, usage_page, usage)
        }
        _ => match kind {
            RIM_TYPEKEYBOARD => (name_vendor, name_product, 0x0001, 0x0006),
            0 => (name_vendor, name_product, 0x0001, 0x0002),
            _ => (name_vendor, name_product, 0, 0),
        },
    };
    let vendor = if vendor == 0 { name_vendor } else { vendor };
    let product = if product == 0 { name_product } else { product };
    let kind = match kind {
        0 => RawKind::Mouse,
        RIM_TYPEKEYBOARD => RawKind::Keyboard,
        RIM_TYPEHID => RawKind::Hid,
        other => RawKind::Other(other),
    };
    Some(RawDevice {
        handle,
        kind,
        vendor,
        product,
        usage_page,
        usage,
        name,
    })
}

fn device_name(device: *mut std::ffi::c_void) -> String {
    let mut chars = 0u32;
    unsafe {
        GetRawInputDeviceInfoW(device, RIDI_DEVICENAME, std::ptr::null_mut(), &mut chars);
    }
    if chars == 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; chars as usize];
    let written = unsafe {
        GetRawInputDeviceInfoW(
            device,
            RIDI_DEVICENAME,
            buffer.as_mut_ptr() as *mut std::ffi::c_void,
            &mut chars,
        )
    };
    if written == u32::MAX || written == 0 {
        return String::new();
    }
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

fn device_info(device: *mut std::ffi::c_void) -> Option<(u32, [u8; 24])> {
    let mut info = DeviceInfo {
        cb_size: std::mem::size_of::<DeviceInfo>() as u32,
        kind: 0,
        data: [0; 24],
    };
    let mut bytes = std::mem::size_of::<DeviceInfo>() as u32;
    let written = unsafe {
        GetRawInputDeviceInfoW(
            device,
            RIDI_DEVICEINFO,
            &mut info as *mut DeviceInfo as *mut std::ffi::c_void,
            &mut bytes,
        )
    };
    if written == u32::MAX || written == 0 {
        return None;
    }
    Some((info.kind, info.data))
}

struct RawWindow {
    hwnd: *mut std::ffi::c_void,
    class: Vec<u16>,
    instance: *mut std::ffi::c_void,
}

impl RawWindow {
    fn open(stop_on_ctrl: bool) -> Result<Self, String> {
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        if instance.is_null() {
            return Err(format!("module handle failed ({})", last_error()));
        }
        let class = wide("MagicKeyboardRawInput");
        unsafe { UnregisterClassW(class.as_ptr(), instance) };
        let window = Self {
            hwnd: std::ptr::null_mut(),
            class,
            instance,
        };
        let class_info = WndClass {
            cb_size: std::mem::size_of::<WndClass>() as u32,
            style: 0,
            wnd_proc: Some(wnd_proc),
            cls_extra: 0,
            wnd_extra: 0,
            instance,
            icon: std::ptr::null_mut(),
            cursor: std::ptr::null_mut(),
            background: std::ptr::null_mut(),
            menu: std::ptr::null(),
            class: window.class.as_ptr(),
            icon_sm: std::ptr::null_mut(),
        };
        if unsafe { RegisterClassExW(&class_info) } == 0 {
            return Err(format!("raw window class failed ({})", last_error()));
        }
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                window.class.as_ptr(),
                window.class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                (-3isize) as *mut std::ffi::c_void,
                std::ptr::null_mut(),
                instance,
                std::ptr::null_mut(),
            )
        };
        if hwnd.is_null() {
            unsafe { UnregisterClassW(window.class.as_ptr(), instance) };
            return Err(format!("raw window failed ({})", last_error()));
        }
        RAW_HWND.store(hwnd, Ordering::Release);
        if stop_on_ctrl {
            unsafe { SetConsoleCtrlHandler(Some(ctrl_handler), 1) };
        }
        Ok(Self {
            hwnd,
            class: window.class.clone(),
            instance,
        })
    }

    fn register(&self) -> Result<(), String> {
        let devices = [
            RawInputDevice {
                usage_page: 0x0001,
                usage: 0x0006,
                flags: RIDEV_INPUTSINK,
                target: self.hwnd,
            },
            RawInputDevice {
                usage_page: 0x000C,
                usage: 0x0001,
                flags: RIDEV_INPUTSINK,
                target: self.hwnd,
            },
            RawInputDevice {
                usage_page: 0x0001,
                usage: 0x0080,
                flags: RIDEV_INPUTSINK,
                target: self.hwnd,
            },
            RawInputDevice {
                usage_page: 0x00FF,
                usage: 0,
                flags: RIDEV_INPUTSINK | RIDEV_PAGEONLY,
                target: self.hwnd,
            },
            RawInputDevice {
                usage_page: 0xFF00,
                usage: 0,
                flags: RIDEV_INPUTSINK | RIDEV_PAGEONLY,
                target: self.hwnd,
            },
        ];
        let ok = unsafe {
            RegisterRawInputDevices(
                devices.as_ptr(),
                devices.len() as u32,
                std::mem::size_of::<RawInputDevice>() as u32,
            )
        };
        if ok == 0 {
            return Err(format!("raw input register failed ({})", last_error()));
        }
        Ok(())
    }

    fn pump(
        &self,
        apple: &[RawDevice],
        seconds: Option<u64>,
        options: &Options,
        tx: Option<&mpsc::Sender<String>>,
    ) -> Result<(), String> {
        let deadline =
            seconds.map(|secs| std::time::Instant::now() + std::time::Duration::from_secs(secs));
        if seconds == Some(0) {
            return Ok(());
        }
        let mut translators: BTreeMap<u16, Translator> = BTreeMap::new();
        loop {
            if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
                return Ok(());
            }
            let mut msg = Msg {
                hwnd: std::ptr::null_mut(),
                message: 0,
                wparam: 0,
                lparam: 0,
                time: 0,
                x: 0,
                y: 0,
                private: 0,
            };
            let got = unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) };
            if got == 0 {
                unsafe { MsgWaitForMultipleObjects(0, std::ptr::null(), 0, 200, QS_ALLINPUT) };
                continue;
            }
            if msg.message == WM_QUIT || msg.message == WM_STOP {
                return Ok(());
            }
            if msg.message == WM_INPUT {
                for line in event_lines(msg.lparam, apple, options, &mut translators, tx.is_some())
                {
                    if let Some(tx) = tx {
                        let _ = tx.send(line);
                    } else {
                        println!("{line}");
                    }
                }
                continue;
            }
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

impl Drop for RawWindow {
    fn drop(&mut self) {
        if self.hwnd.is_null() {
            return;
        }
        let current = RAW_HWND.load(Ordering::Acquire);
        if current == self.hwnd {
            RAW_HWND.store(std::ptr::null_mut(), Ordering::Release);
        }
        let remove = [
            RawInputDevice {
                usage_page: 0x0001,
                usage: 0x0006,
                flags: RIDEV_REMOVE,
                target: std::ptr::null_mut(),
            },
            RawInputDevice {
                usage_page: 0x000C,
                usage: 0x0001,
                flags: RIDEV_REMOVE,
                target: std::ptr::null_mut(),
            },
            RawInputDevice {
                usage_page: 0x0001,
                usage: 0x0080,
                flags: RIDEV_REMOVE,
                target: std::ptr::null_mut(),
            },
            RawInputDevice {
                usage_page: 0x00FF,
                usage: 0,
                flags: RIDEV_REMOVE | RIDEV_PAGEONLY,
                target: std::ptr::null_mut(),
            },
            RawInputDevice {
                usage_page: 0xFF00,
                usage: 0,
                flags: RIDEV_REMOVE | RIDEV_PAGEONLY,
                target: std::ptr::null_mut(),
            },
        ];
        unsafe {
            RegisterRawInputDevices(
                remove.as_ptr(),
                remove.len() as u32,
                std::mem::size_of::<RawInputDevice>() as u32,
            );
            DestroyWindow(self.hwnd);
            UnregisterClassW(self.class.as_ptr(), self.instance);
        }
    }
}

fn event_lines(
    handle: isize,
    apple: &[RawDevice],
    options: &Options,
    translators: &mut BTreeMap<u16, Translator>,
    scans_only: bool,
) -> Vec<String> {
    let Ok(bytes) = read_input(handle) else {
        return Vec::new();
    };
    if bytes.len() < std::mem::size_of::<Header>() {
        return Vec::new();
    }
    let Some(kind_bytes) = bytes.get(..4) else {
        return Vec::new();
    };
    let Ok(kind_bytes) = kind_bytes.try_into() else {
        return Vec::new();
    };
    let kind = u32::from_le_bytes(kind_bytes);
    let device_at = std::mem::offset_of!(Header, device);
    let Some(device_bytes) = bytes.get(device_at..device_at + std::mem::size_of::<usize>()) else {
        return Vec::new();
    };
    let Ok(device_bytes) = device_bytes.try_into() else {
        return Vec::new();
    };
    let device = usize::from_le_bytes(device_bytes);
    let matched = apple.iter().find(|item| item.handle == device);
    let name = device_name(device as *mut std::ffi::c_void);
    let (vendor, product) = parse_hid_ids(&name).unwrap_or((0, 0));
    let product = if product != 0 {
        product
    } else {
        matched.map(|item| item.product).unwrap_or(0)
    };
    let vendor = if vendor != 0 {
        vendor
    } else {
        matched.map(|item| item.vendor).unwrap_or(0)
    };
    if !apple_device(vendor, product) {
        return Vec::new();
    }
    let data = &bytes[std::mem::size_of::<Header>()..];
    if kind == RIM_TYPEKEYBOARD {
        return format_keyboard(product, data).into_iter().collect();
    }
    if scans_only || kind != RIM_TYPEHID {
        return Vec::new();
    }
    let usage_page = matched.map(|item| item.usage_page).unwrap_or(0);
    let usage = matched.map(|item| item.usage).unwrap_or(0);
    translate_hid(
        vendor,
        product,
        usage_page,
        usage,
        data,
        options,
        translators,
    )
}

fn translate_hid(
    vendor: u16,
    product: u16,
    usage_page: u16,
    usage: u16,
    data: &[u8],
    options: &Options,
    translators: &mut BTreeMap<u16, Translator>,
) -> Vec<String> {
    let Some(bytes) = hid_bytes(data) else {
        return vec![format_hid(product, usage_page, usage, data)];
    };
    let Some(report) = parse_report(bytes) else {
        return vec![format!(
            "hid product {product:04x} usage {usage_page:04x}:{usage:04x} {}",
            hex_bytes(bytes)
        )];
    };
    let Some(translator) = translator_for(translators, vendor, product, options) else {
        return vec![format!(
            "hid product {product:04x} usage {usage_page:04x}:{usage:04x} {}",
            hex_bytes(bytes)
        )];
    };
    translator
        .push(&report)
        .into_iter()
        .map(|line| format!("{product:04x} {line}"))
        .collect()
}

fn translator_for<'a>(
    translators: &'a mut BTreeMap<u16, Translator>,
    vendor: u16,
    product: u16,
    options: &Options,
) -> Option<&'a mut Translator> {
    if translators.contains_key(&product) {
        return translators.get_mut(&product);
    }
    let model = identify(vendor, product)?;
    translators.insert(
        product,
        Translator::new(model.map, model.iso_quirk, *options),
    );
    translators.get_mut(&product)
}

fn hid_bytes(data: &[u8]) -> Option<&[u8]> {
    if data.len() < 8 {
        return None;
    }
    let size = u32::from_le_bytes(data[0..4].try_into().ok()?) as usize;
    let count = u32::from_le_bytes(data[4..8].try_into().ok()?) as usize;
    let take = size.saturating_mul(count).min(data.len().saturating_sub(8));
    Some(&data[8..8 + take])
}

fn read_input(handle: isize) -> Result<Vec<u8>, u32> {
    let header = std::mem::size_of::<Header>() as u32;
    let mut size = 0u32;
    let rc = unsafe { GetRawInputData(handle, RID_INPUT, std::ptr::null_mut(), &mut size, header) };
    if rc == u32::MAX || size == 0 {
        return Err(last_error());
    }
    let mut buffer = vec![0u8; size as usize];
    let written = unsafe {
        GetRawInputData(
            handle,
            RID_INPUT,
            buffer.as_mut_ptr() as *mut std::ffi::c_void,
            &mut size,
            header,
        )
    };
    if written == u32::MAX {
        return Err(last_error());
    }
    buffer.truncate(written as usize);
    Ok(buffer)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn last_error() -> u32 {
    unsafe { GetLastError() }
}

unsafe extern "system" fn wnd_proc(
    hwnd: *mut std::ffi::c_void,
    msg: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe extern "system" fn ctrl_handler(code: u32) -> i32 {
    if code == 0 || code == 1 {
        let hwnd = RAW_HWND.load(Ordering::Acquire);
        if !hwnd.is_null() {
            PostMessageW(hwnd, WM_STOP, 0, 0);
        }
        1
    } else {
        0
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn GetLastError() -> u32;
    fn GetModuleHandleW(name: *const u16) -> *mut std::ffi::c_void;
    fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn GetRawInputDeviceList(list: *mut DeviceEntry, count: *mut u32, size: u32) -> u32;
    fn GetRawInputDeviceInfoW(
        device: *mut std::ffi::c_void,
        command: u32,
        data: *mut std::ffi::c_void,
        size: *mut u32,
    ) -> u32;
    fn GetRawInputData(
        handle: isize,
        command: u32,
        data: *mut std::ffi::c_void,
        size: *mut u32,
        header: u32,
    ) -> u32;
    fn RegisterRawInputDevices(devices: *const RawInputDevice, count: u32, size: u32) -> i32;
    fn RegisterClassExW(class: *const WndClass) -> u16;
    fn UnregisterClassW(class: *const u16, instance: *mut std::ffi::c_void) -> i32;
    fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        window: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: *mut std::ffi::c_void,
        menu: *mut std::ffi::c_void,
        instance: *mut std::ffi::c_void,
        param: *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_void;
    fn DestroyWindow(hwnd: *mut std::ffi::c_void) -> i32;
    fn DefWindowProcW(hwnd: *mut std::ffi::c_void, msg: u32, wparam: usize, lparam: isize)
        -> isize;
    fn PeekMessageW(
        msg: *mut Msg,
        hwnd: *mut std::ffi::c_void,
        min: u32,
        max: u32,
        remove: u32,
    ) -> i32;
    fn TranslateMessage(msg: *const Msg) -> i32;
    fn DispatchMessageW(msg: *const Msg) -> isize;
    fn PostMessageW(hwnd: *mut std::ffi::c_void, msg: u32, wparam: usize, lparam: isize) -> i32;
    fn MsgWaitForMultipleObjects(
        count: u32,
        handles: *const std::ffi::c_void,
        wait_all: i32,
        millis: u32,
        wake: u32,
    ) -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usb_and_bluetooth_hardware_ids() {
        let usb = r"\\?\HID#VID_05AC&PID_0321&MI_01&Col01#instance#{guid}";
        assert_eq!(parse_hid_ids(usb), Some((0x05AC, 0x0321)));
        let bluetooth =
            r"\\?\HID#{00001124-0000-1000-8000-00805f9b34fb}_VID&0001004c_PID&0267#node";
        assert_eq!(parse_hid_ids(bluetooth), Some((0x004C, 0x0267)));
    }

    #[test]
    fn list_names_the_keyboard_collection_as_scan_codes() {
        let text = format_raw_list(
            &[RawDevice {
                handle: 1,
                kind: RawKind::Keyboard,
                vendor: 0x05AC,
                product: 0x0321,
                usage_page: 0x0001,
                usage: 0x0006,
                name: r"\\?\HID#VID_05AC&PID_0321&MI_01&Col01#instance".to_string(),
            }],
            4,
        );
        assert!(text.contains("keyboard 05ac:0321 usage 0001:0006"));
        assert!(text.contains("Magic Keyboard with Touch ID (2024)"));
        assert!(text.contains("4 raw devices"));
    }

    #[test]
    fn keyboard_events_are_scan_codes_and_hid_events_keep_fn() {
        let down = [0x1E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x41, 0x00, 0, 0, 0, 0];
        assert_eq!(format_keyboard(0x0321, &down).unwrap(), "scan 0321 down A");
        let mut hid = vec![10u8, 0, 0, 0, 1, 0, 0, 0];
        hid.extend_from_slice(&[0x01, 0x00, 0x00, 0x3A, 0, 0, 0, 0, 0, 0x02]);
        let mut translators = BTreeMap::new();
        let lines = translate_hid(
            0x05AC,
            0x0321,
            0x0001,
            0x0006,
            &hid,
            &Options::default(),
            &mut translators,
        );
        assert_eq!(
            lines,
            vec!["0321 down fn".to_string(), "0321 down F1".to_string()]
        );
    }

    #[test]
    fn header_layout_matches_the_raw_input_struct() {
        if cfg!(target_pointer_width = "64") {
            assert_eq!(std::mem::size_of::<Header>(), 24);
            assert_eq!(std::mem::size_of::<DeviceEntry>(), 16);
            assert_eq!(std::mem::size_of::<DeviceInfo>(), 32);
            assert_eq!(std::mem::size_of::<WndClass>(), 80);
            assert_eq!(std::mem::size_of::<Msg>(), 48);
        }
    }
}
