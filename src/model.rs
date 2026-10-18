//! Apple Magic Keyboard identities and the Fn maps for each generation.
//!
//! Product ids are the ones Linux `hid-apple` matches, and the same ids the
//! USB device enumerates with. `iso_quirk` is the ISO tilde quirk: every
//! Magic Keyboard from 2015 on, plus the aluminum wireless ISO models.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapKind {
    /// 2007-2011 aluminum wireless keyboards. F6 is Num Lock.
    Wireless,
    /// 2015 keyboard and the 2017 keypad model. F5 and F6 stay function keys.
    Magic2015,
    /// 2021 and 2024 keyboards, with or without Touch ID or a keypad.
    Magic2021,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Model {
    pub product: u16,
    pub name: &'static str,
    pub touch_id: bool,
    pub numpad: bool,
    /// Linux ISO tilde quirk. Auto grave swap also needs HID country code 13.
    pub iso_quirk: bool,
    pub map: MapKind,
}

const MODELS: &[Model] = &[
    model(
        0x022C,
        "Apple Wireless Keyboard (ANSI)",
        false,
        false,
        false,
        MapKind::Wireless,
    ),
    model(
        0x022D,
        "Apple Wireless Keyboard (ISO)",
        false,
        false,
        true,
        MapKind::Wireless,
    ),
    model(
        0x022E,
        "Apple Wireless Keyboard (JIS)",
        false,
        false,
        false,
        MapKind::Wireless,
    ),
    model(
        0x0239,
        "Apple Wireless Keyboard 2009 (ANSI)",
        false,
        false,
        false,
        MapKind::Wireless,
    ),
    model(
        0x023A,
        "Apple Wireless Keyboard 2009 (ISO)",
        false,
        false,
        true,
        MapKind::Wireless,
    ),
    model(
        0x023B,
        "Apple Wireless Keyboard 2009 (JIS)",
        false,
        false,
        false,
        MapKind::Wireless,
    ),
    model(
        0x0255,
        "Apple Wireless Keyboard 2011 (ANSI)",
        false,
        false,
        false,
        MapKind::Wireless,
    ),
    model(
        0x0256,
        "Apple Wireless Keyboard 2011 (ISO)",
        false,
        false,
        true,
        MapKind::Wireless,
    ),
    model(
        0x0257,
        "Apple Wireless Keyboard 2011 (JIS)",
        false,
        false,
        false,
        MapKind::Wireless,
    ),
    model(
        0x0267,
        "Magic Keyboard (2015)",
        false,
        false,
        true,
        MapKind::Magic2015,
    ),
    model(
        0x026C,
        "Magic Keyboard with Numeric Keypad (2017)",
        false,
        true,
        true,
        MapKind::Magic2015,
    ),
    model(
        0x029C,
        "Magic Keyboard (2021)",
        false,
        false,
        true,
        MapKind::Magic2021,
    ),
    model(
        0x029A,
        "Magic Keyboard with Touch ID (2021)",
        true,
        false,
        true,
        MapKind::Magic2021,
    ),
    model(
        0x029F,
        "Magic Keyboard with Touch ID and Numeric Keypad (2021)",
        true,
        true,
        true,
        MapKind::Magic2021,
    ),
    model(
        0x0320,
        "Magic Keyboard (2024)",
        false,
        false,
        true,
        MapKind::Magic2021,
    ),
    model(
        0x0321,
        "Magic Keyboard with Touch ID (2024)",
        true,
        false,
        true,
        MapKind::Magic2021,
    ),
    model(
        0x0322,
        "Magic Keyboard with Touch ID and Numeric Keypad (2024)",
        true,
        true,
        true,
        MapKind::Magic2021,
    ),
];

const fn model(
    product: u16,
    name: &'static str,
    touch_id: bool,
    numpad: bool,
    iso_quirk: bool,
    map: MapKind,
) -> Model {
    Model {
        product,
        name,
        touch_id,
        numpad,
        iso_quirk,
        map,
    }
}

/// USB Magic Keyboards from 2015 onward have a battery report. Bluetooth copies do not.
pub fn usb_battery(vendor: u16, product: u16) -> bool {
    vendor == 0x05AC
        && identify(vendor, product)
            .is_some_and(|model| matches!(model.map, MapKind::Magic2015 | MapKind::Magic2021))
}

/// USB vendor is `0x05AC`. Bluetooth HID uses `0x004C` with the same product id.
pub fn identify(vendor: u16, product: u16) -> Option<&'static Model> {
    if vendor != 0x05AC && vendor != 0x004C {
        return None;
    }
    MODELS.iter().find(|model| model.product == product)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsbInterface {
    pub number: u8,
    pub role: &'static str,
    pub transfer: &'static str,
    pub packet: u16,
    pub report_descriptor_len: u16,
}

/// USB configuration read from a connected Magic Keyboard with Touch ID (2024).
pub fn measured_interfaces(product: u16) -> Option<&'static [UsbInterface]> {
    if product != 0x0321 {
        return None;
    }
    Some(&[
        UsbInterface {
            number: 0,
            role: "device management",
            transfer: "interrupt in",
            packet: 16,
            report_descriptor_len: 263,
        },
        UsbInterface {
            number: 1,
            role: "keyboard",
            transfer: "interrupt in",
            packet: 16,
            report_descriptor_len: 207,
        },
        UsbInterface {
            number: 2,
            role: "touch id",
            transfer: "bulk in",
            packet: 64,
            report_descriptor_len: 49,
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_the_connected_keyboard() {
        let model = identify(0x05AC, 0x0321).unwrap();
        assert_eq!(model.name, "Magic Keyboard with Touch ID (2024)");
        assert!(model.touch_id);
        assert_eq!(model.map, MapKind::Magic2021);
        assert!(identify(0x004C, 0x0321).is_some());
        assert!(identify(0x046D, 0x0321).is_none());
        assert!(identify(0x05AC, 0x0265).is_none());
    }

    #[test]
    fn iso_quirk_follows_the_linux_magic_keyboard_table() {
        for product in [
            0x0267, 0x026C, 0x029C, 0x029A, 0x029F, 0x0320, 0x0321, 0x0322,
        ] {
            assert!(
                identify(0x05AC, product).unwrap().iso_quirk,
                "{product:04x}"
            );
        }
        for product in [0x022D, 0x023A, 0x0256] {
            assert!(
                identify(0x05AC, product).unwrap().iso_quirk,
                "{product:04x}"
            );
        }
        for product in [0x022C, 0x022E, 0x0239, 0x023B, 0x0255, 0x0257] {
            assert!(
                !identify(0x05AC, product).unwrap().iso_quirk,
                "{product:04x}"
            );
        }
    }

    #[test]
    fn battery_quirk_is_usb_only_from_2015() {
        assert!(usb_battery(0x05AC, 0x0321));
        assert!(usb_battery(0x05AC, 0x0267));
        assert!(!usb_battery(0x004C, 0x0321));
        assert!(!usb_battery(0x05AC, 0x022D));
    }

    #[test]
    fn measured_topology_matches_the_config_descriptor() {
        let ifaces = measured_interfaces(0x0321).unwrap();
        assert_eq!(ifaces.len(), 3);
        assert_eq!(ifaces[0].report_descriptor_len, 263);
        assert_eq!(ifaces[1].role, "keyboard");
        assert_eq!(ifaces[2].transfer, "bulk in");
    }
}
