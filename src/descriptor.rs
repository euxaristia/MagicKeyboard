//! Report-descriptor repairs for Magic Keyboard shapes Linux recognizes.
//!
//! The JIS repair raises two logical-maximum bytes. The battery repair turns
//! an 83-byte vendor collection into a Generic Desktop keyboard usage. The
//! JIS bytes are rewritten first so the battery length check still sees the
//! original indexes.

/// Copy `descriptor`, applying the Magic Keyboard JIS repair and then the
/// 83-byte battery repair when their marker bytes are present.
pub fn fix_magic_keyboard_descriptor(descriptor: &[u8]) -> Vec<u8> {
    let mut out = descriptor.to_vec();
    if out.len() >= 71 && out[70] == 0x65 && out[64] == 0x65 {
        out[64] = 0xe7;
        out[70] = 0xe7;
    }
    if out.len() == 83 && out[46] == 0x84 && out[58] == 0x85 {
        out.remove(0);
        out[0] = 0x05;
        out[1] = 0x01;
        out[2] = 0x09;
        out[3] = 0x06;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jis_logical_maximum_bytes_become_e7() {
        let mut descriptor = vec![0u8; 71];
        descriptor[10] = 0x11;
        descriptor[64] = 0x65;
        descriptor[70] = 0x65;
        let fixed = fix_magic_keyboard_descriptor(&descriptor);
        assert_eq!(fixed.len(), 71);
        assert_eq!(fixed[64], 0xe7);
        assert_eq!(fixed[70], 0xe7);
        assert_eq!(fixed[10], 0x11);
    }

    #[test]
    fn battery_collection_drops_the_vendor_page_prefix() {
        let mut descriptor = vec![0u8; 83];
        descriptor[0] = 0x06;
        descriptor[6] = 0xab;
        descriptor[46] = 0x84;
        descriptor[58] = 0x85;
        let fixed = fix_magic_keyboard_descriptor(&descriptor);
        assert_eq!(fixed.len(), 82);
        assert_eq!(&fixed[..4], &[0x05, 0x01, 0x09, 0x06]);
        assert_eq!(fixed[5], 0xab);
    }

    #[test]
    fn jis_repair_runs_before_the_battery_length_check() {
        let mut descriptor = vec![0u8; 83];
        descriptor[46] = 0x84;
        descriptor[58] = 0x85;
        descriptor[64] = 0x65;
        descriptor[70] = 0x65;
        let fixed = fix_magic_keyboard_descriptor(&descriptor);
        assert_eq!(fixed.len(), 82);
        assert_eq!(&fixed[..4], &[0x05, 0x01, 0x09, 0x06]);
        assert_eq!(fixed[63], 0xe7);
        assert_eq!(fixed[69], 0xe7);
    }

    #[test]
    fn other_descriptors_pass_through() {
        let short = vec![0x65u8; 70];
        assert_eq!(fix_magic_keyboard_descriptor(&short), short);

        let mut macbook = vec![0u8; 60];
        macbook[53] = 0x65;
        macbook[59] = 0x65;
        assert_eq!(fix_magic_keyboard_descriptor(&macbook), macbook);

        let mut almost = vec![0u8; 83];
        almost[46] = 0x84;
        assert_eq!(fix_magic_keyboard_descriptor(&almost), almost);
    }
}
