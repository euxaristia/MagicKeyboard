mod descriptor;
mod model;
mod names;
mod present;
mod report;
mod session;

pub use descriptor::fix_magic_keyboard_descriptor;
pub use model::{identify, measured_interfaces, usb_battery, MapKind, Model, UsbInterface};
pub use present::{format_status, CollectionView, DeviceView};
pub use report::{parse_keyboard, parse_report, InputReport, KeyboardReport};
pub use session::{Action, Edge, FnMode, Iso, Options, Session, SwapOptCmd};

#[cfg(windows)]
mod winhid;

#[cfg(windows)]
pub use winhid::{listen, status};
