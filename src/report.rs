//! HID input reports from the keyboard boot interface.
//!
//! Layout was taken from HidP on the connected 2024 Touch ID keyboard, not
//! from a guessed descriptor. Report 1 is 10 bytes:
//!
//! ```text
//! 0     report id 0x01
//! 1     modifier bitmask, usage 0xE0 at bit 0 through 0xE7 at bit 7
//! 2     reserved
//! 3..8  six keyboard usages, 0 when empty
//! 9     bit 0 eject (consumer 0x00B8), bit 1 Fn (vendor page 0x00FF usage 0x0003)
//! ```
//!
//! Report 3 is the consumer collection: id, then a little-endian usage.
//! Volume increment was `03 E9 00`. Report 4 is system control: id, then
//! `01` when System Sleep (usage 0x0082) is down.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyboardReport {
    pub modifiers: u8,
    pub keys: [u8; 6],
    pub eject: bool,
    pub function: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConsumerReport {
    pub usage: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemReport {
    pub sleep: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputReport {
    Keyboard(KeyboardReport),
    Consumer(ConsumerReport),
    System(SystemReport),
}

pub fn parse_report(bytes: &[u8]) -> Option<InputReport> {
    match bytes.first().copied()? {
        0x01 => parse_keyboard(bytes).map(InputReport::Keyboard),
        0x03 => parse_consumer(bytes).map(InputReport::Consumer),
        0x04 => parse_system(bytes).map(InputReport::System),
        _ => None,
    }
}

pub fn parse_keyboard(bytes: &[u8]) -> Option<KeyboardReport> {
    if bytes.len() < 10 || bytes[0] != 0x01 {
        return None;
    }
    let mut keys = [0u8; 6];
    keys.copy_from_slice(&bytes[3..9]);
    // Error rollover fills every slot with 0x01. That is not six real keys.
    if keys == [0x01; 6] {
        keys = [0; 6];
    }
    let flags = bytes[9];
    Some(KeyboardReport {
        modifiers: bytes[1],
        keys,
        eject: flags & 0x01 != 0,
        function: flags & 0x02 != 0,
    })
}

pub fn parse_consumer(bytes: &[u8]) -> Option<ConsumerReport> {
    if bytes.len() < 3 || bytes[0] != 0x03 {
        return None;
    }
    Some(ConsumerReport {
        usage: u16::from_le_bytes([bytes[1], bytes[2]]),
    })
}

pub fn parse_system(bytes: &[u8]) -> Option<SystemReport> {
    if bytes.len() < 2 || bytes[0] != 0x04 {
        return None;
    }
    Some(SystemReport {
        sleep: bytes[1] & 0x01 != 0,
    })
}

/// Usages currently down, modifiers first, then the six slots. Zeros omitted.
pub fn wire_usages(report: &KeyboardReport) -> Vec<u16> {
    let mut usages = Vec::with_capacity(14);
    for bit in 0..8 {
        if report.modifiers & (1 << bit) != 0 {
            usages.push(0xE0 + bit);
        }
    }
    for key in report.keys {
        if key != 0 {
            usages.push(u16::from(key));
        }
    }
    usages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_live_hidp_patterns() {
        let report = parse_keyboard(&[0x01, 0x02, 0x00, 0x04, 0x05, 0, 0, 0, 0, 0x03]).unwrap();
        assert_eq!(report.modifiers, 0x02);
        assert_eq!(&report.keys[..2], &[0x04, 0x05]);
        assert!(report.eject);
        assert!(report.function);
        assert_eq!(wire_usages(&report), vec![0xE1, 0x04, 0x05]);
    }

    #[test]
    fn drops_error_rollover() {
        let report = parse_keyboard(&[0x01, 0, 0, 1, 1, 1, 1, 1, 1, 0]).unwrap();
        assert_eq!(report.keys, [0; 6]);
        assert!(wire_usages(&report).is_empty());
    }

    #[test]
    fn parses_consumer_and_system_reports() {
        assert_eq!(parse_consumer(&[0x03, 0xE9, 0x00]).unwrap().usage, 0x00E9);
        assert!(parse_system(&[0x04, 0x01]).unwrap().sleep);
        assert!(!parse_system(&[0x04, 0x00]).unwrap().sleep);
        assert!(parse_report(&[0x02, 0]).is_none());
    }
}
