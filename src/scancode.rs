//! Set 1 scan codes from Raw Input, named as HID keyboard usages.
//!
//! This is the boot collection after the inbox keyboard driver has taken it.
//! The Fn bit is not in these events, so they are not run through the Fn map.

use crate::names::label;
use crate::session::Action;

const BREAK: u16 = 0x0001;
const EXTENDED: u16 = 0x0002;
const PAUSE: u16 = 0x0004;

pub fn format_scan(product: u16, make: u16, flags: u16) -> String {
    let dir = if flags & BREAK == 0 { "down" } else { "up" };
    let name = if flags & PAUSE != 0 {
        None
    } else {
        usage_from_scan(make, flags & EXTENDED != 0).map(|usage| label(&Action::Key(usage)))
    };
    let name = name.unwrap_or_else(|| format!("make {make:04x}"));
    format!("scan {product:04x} {dir} {name}")
}

pub fn usage_from_scan(make: u16, extended: bool) -> Option<u16> {
    if extended {
        return match make {
            0x1C => Some(0x58),
            0x1D => Some(0xE4),
            0x35 => Some(0x54),
            0x38 => Some(0xE6),
            0x47 => Some(0x4A),
            0x48 => Some(0x52),
            0x49 => Some(0x4B),
            0x4B => Some(0x50),
            0x4D => Some(0x4F),
            0x4F => Some(0x4D),
            0x50 => Some(0x51),
            0x51 => Some(0x4E),
            0x52 => Some(0x49),
            0x53 => Some(0x4C),
            0x5B => Some(0xE3),
            0x5C => Some(0xE7),
            _ => None,
        };
    }
    match make {
        0x01 => Some(0x29),
        0x02 => Some(0x1E),
        0x03 => Some(0x1F),
        0x04 => Some(0x20),
        0x05 => Some(0x21),
        0x06 => Some(0x22),
        0x07 => Some(0x23),
        0x08 => Some(0x24),
        0x09 => Some(0x25),
        0x0A => Some(0x26),
        0x0B => Some(0x27),
        0x0C => Some(0x2D),
        0x0D => Some(0x2E),
        0x0E => Some(0x2A),
        0x0F => Some(0x2B),
        0x10 => Some(0x14),
        0x11 => Some(0x1A),
        0x12 => Some(0x08),
        0x13 => Some(0x15),
        0x14 => Some(0x17),
        0x15 => Some(0x1C),
        0x16 => Some(0x18),
        0x17 => Some(0x0C),
        0x18 => Some(0x12),
        0x19 => Some(0x13),
        0x1A => Some(0x2F),
        0x1B => Some(0x30),
        0x1C => Some(0x28),
        0x1D => Some(0xE0),
        0x1E => Some(0x04),
        0x1F => Some(0x16),
        0x20 => Some(0x07),
        0x21 => Some(0x09),
        0x22 => Some(0x0A),
        0x23 => Some(0x0B),
        0x24 => Some(0x0D),
        0x25 => Some(0x0E),
        0x26 => Some(0x0F),
        0x27 => Some(0x33),
        0x28 => Some(0x34),
        0x29 => Some(0x35),
        0x2A => Some(0xE1),
        0x2B => Some(0x31),
        0x2C => Some(0x1D),
        0x2D => Some(0x1B),
        0x2E => Some(0x06),
        0x2F => Some(0x19),
        0x30 => Some(0x05),
        0x31 => Some(0x11),
        0x32 => Some(0x10),
        0x33 => Some(0x36),
        0x34 => Some(0x37),
        0x35 => Some(0x38),
        0x36 => Some(0xE5),
        0x38 => Some(0xE2),
        0x39 => Some(0x2C),
        0x3A => Some(0x39),
        0x3B => Some(0x3A),
        0x3C => Some(0x3B),
        0x3D => Some(0x3C),
        0x3E => Some(0x3D),
        0x3F => Some(0x3E),
        0x40 => Some(0x3F),
        0x41 => Some(0x40),
        0x42 => Some(0x41),
        0x43 => Some(0x42),
        0x44 => Some(0x43),
        0x57 => Some(0x44),
        0x58 => Some(0x45),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_letter_arrow_and_leaves_an_unknown_make() {
        assert_eq!(format_scan(0x0321, 0x1E, 0), "scan 0321 down A");
        assert_eq!(format_scan(0x0321, 0x1E, BREAK), "scan 0321 up A");
        assert_eq!(format_scan(0x0321, 0x4B, EXTENDED), "scan 0321 down left");
        assert_eq!(
            format_scan(0x0321, 0x5B, EXTENDED),
            "scan 0321 down left cmd"
        );
        assert_eq!(format_scan(0x0321, 0x63, 0), "scan 0321 down make 0063");
    }
}
