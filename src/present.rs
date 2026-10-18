use crate::model::{identify, measured_interfaces, MapKind};
use crate::names::{consumer_name, label};
use crate::report::InputReport;
use crate::session::{Action, Options, Session};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionView {
    pub usage_page: u16,
    pub usage: u16,
    pub input_len: u16,
    pub report: Option<InputReport>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceView {
    pub vendor: u16,
    pub product: u16,
    pub serial: String,
    pub manufacturer: String,
    pub product_name: String,
    pub collections: Vec<CollectionView>,
}

pub fn format_status(devices: &[DeviceView]) -> String {
    let mut out = String::new();
    if devices.is_empty() {
        return "no Magic Keyboard found\n".to_string();
    }
    for device in devices {
        let model = identify(device.vendor, device.product);
        let name = model.map(|model| model.name).unwrap_or("Magic Keyboard");
        out.push_str(name);
        out.push('\n');
        out.push_str(&format!("  {:04x}:{:04x}", device.vendor, device.product));
        if !device.serial.is_empty() {
            out.push_str("  serial ");
            out.push_str(&device.serial);
        }
        out.push('\n');
        if !device.manufacturer.is_empty() || !device.product_name.is_empty() {
            out.push_str("  ");
            out.push_str(&device.manufacturer);
            if !device.product_name.is_empty() {
                if !device.manufacturer.is_empty() {
                    out.push_str(" / ");
                }
                out.push_str(&device.product_name);
            }
            out.push('\n');
        }
        if let Some(interfaces) = measured_interfaces(device.product) {
            for interface in interfaces {
                out.push_str(&format!(
                    "  usb {} {}: {}, {}-byte packets, {}-byte report descriptor\n",
                    interface.number,
                    interface.role,
                    interface.transfer,
                    interface.packet,
                    interface.report_descriptor_len
                ));
            }
        }
        for collection in &device.collections {
            out.push_str(&format!(
                "  hid {}, {}-byte input\n",
                collection_name(collection.usage_page, collection.usage),
                collection.input_len
            ));
            match &collection.report {
                Some(InputReport::Keyboard(report)) => {
                    let fn_state = if report.function { "down" } else { "up" };
                    let eject = if report.eject { "down" } else { "up" };
                    out.push_str(&format!(
                        "  idle: fn {fn_state}, eject {eject}, modifiers {:02x}\n",
                        report.modifiers
                    ));
                }
                None if collection.usage_page == 0x0001 && collection.usage == 0x0006 => {
                    out.push_str("  idle report not available\n");
                }
                _ => {}
            }
        }
        if model.is_some_and(|model| model.touch_id) {
            out.push_str("  Touch ID authenticates only to a paired Mac and is not driven\n");
        }
        if measured_interfaces(device.product).is_some() {
            out.push_str(
                "  battery is on usb interface 0, which has no HID function driver bound\n",
            );
        }
    }
    out
}

pub fn collection_name(usage_page: u16, usage: u16) -> &'static str {
    match (usage_page, usage) {
        (0x0001, 0x0006) => "keyboard",
        (0x000C, 0x0001) => "consumer control",
        (0x0001, 0x0080) => "system control",
        _ => "vendor collection",
    }
}

pub fn edge_line(action: &Action, down: bool) -> String {
    let dir = if down { "down" } else { "up" };
    format!("{dir} {}", label(action))
}

/// Keyboard, consumer, and system reports in one place. Keyboard reports go
/// through the Fn map. Consumer and system reports become down and up edges.
pub struct Translator {
    session: Session,
    consumer: u16,
    sleep: bool,
}

impl Translator {
    pub fn new(map: MapKind, iso_quirk: bool, options: Options) -> Self {
        Self {
            session: Session::new(map, iso_quirk, None, options),
            consumer: 0,
            sleep: false,
        }
    }

    pub fn push(&mut self, report: &InputReport) -> Vec<String> {
        match report {
            InputReport::Keyboard(report) => self
                .session
                .push_keyboard(report)
                .iter()
                .map(|edge| edge_line(&edge.action, edge.down))
                .collect(),
            InputReport::Consumer(report) => self.push_consumer(report.usage),
            InputReport::System(report) => self.push_sleep(report.sleep),
        }
    }

    fn push_consumer(&mut self, usage: u16) -> Vec<String> {
        if usage == self.consumer {
            return Vec::new();
        }
        let mut lines = Vec::new();
        if self.consumer != 0 {
            lines.push(format!("up {}", consumer_name(self.consumer)));
        }
        self.consumer = usage;
        if usage != 0 {
            lines.push(format!("down {}", consumer_name(usage)));
        }
        lines
    }

    fn push_sleep(&mut self, sleep: bool) -> Vec<String> {
        if sleep == self.sleep {
            return Vec::new();
        }
        self.sleep = sleep;
        vec![edge_line(&Action::Sleep, sleep)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{parse_keyboard, InputReport};

    #[test]
    fn status_describes_the_2024_touch_id_keyboard() {
        let keyboard = parse_keyboard(&[0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        let text = format_status(&[DeviceView {
            vendor: 0x05AC,
            product: 0x0321,
            serial: "EXAMPLE".to_string(),
            manufacturer: "Apple Inc.".to_string(),
            product_name: "Keyboard / Boot".to_string(),
            collections: vec![CollectionView {
                usage_page: 0x0001,
                usage: 0x0006,
                input_len: 10,
                report: Some(InputReport::Keyboard(keyboard)),
            }],
        }]);
        assert!(text.contains("Magic Keyboard with Touch ID (2024)"));
        assert!(text.contains("05ac:0321"));
        assert!(text.contains("usb 0 device management"));
        assert!(text.contains("usb 2 touch id"));
        assert!(text.contains("idle: fn up, eject up"));
        assert!(text.contains("not driven"));
        assert!(text.contains("battery is on usb interface 0"));
    }

    #[test]
    fn one_translator_covers_keyboard_consumer_and_sleep() {
        use crate::report::{parse_consumer, parse_keyboard, parse_system};
        use crate::session::FnMode;

        let mut translator = Translator::new(
            MapKind::Magic2021,
            true,
            Options {
                fn_mode: FnMode::FkeysLast,
                ..Options::default()
            },
        );
        let keyboard = parse_keyboard(&[0x01, 0, 0, 0x3A, 0, 0, 0, 0, 0, 0x02]).unwrap();
        assert_eq!(
            translator.push(&InputReport::Keyboard(keyboard)),
            vec!["down fn".to_string(), "down F1".to_string()]
        );
        let volume = parse_consumer(&[0x03, 0xE9, 0x00]).unwrap();
        assert_eq!(
            translator.push(&InputReport::Consumer(volume)),
            vec!["down volume up".to_string()]
        );
        let release = parse_consumer(&[0x03, 0x00, 0x00]).unwrap();
        assert_eq!(
            translator.push(&InputReport::Consumer(release)),
            vec!["up volume up".to_string()]
        );
        let sleep = parse_system(&[0x04, 0x01]).unwrap();
        assert_eq!(
            translator.push(&InputReport::System(sleep)),
            vec!["down sleep".to_string()]
        );
    }
}
