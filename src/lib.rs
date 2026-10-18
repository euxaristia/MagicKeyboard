mod descriptor;
mod inject;
mod model;
mod names;
mod present;
mod report;
mod scancode;
mod session;

pub use descriptor::fix_magic_keyboard_descriptor;
pub use model::{identify, measured_interfaces, usb_battery, MapKind, Model, UsbInterface};
pub use present::{format_status, CollectionView, DeviceView, Translator};
pub use report::{parse_keyboard, parse_report, InputReport, KeyboardReport};
pub use scancode::format_scan;
pub use session::{Action, Edge, FnMode, Iso, Options, Session, SwapOptCmd};

#[cfg(windows)]
mod rawinput;
#[cfg(windows)]
mod winhid;
#[cfg(windows)]
mod winusb;

#[cfg(windows)]
pub use rawinput::{listen_raw, raw_status};
#[cfg(windows)]
pub use winhid::{listen, status};
#[cfg(windows)]
pub use winusb::drive;
