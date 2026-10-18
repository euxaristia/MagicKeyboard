# magickeyboard

Userspace driver for Apple Magic Keyboard HID reports. It reads the boot keyboard, consumer, and system collections and translates the Fn layer the way Linux `hid-apple` does (`drivers/hid/hid-apple.c`). The Fn tables and the modifier-swap order follow that driver. This crate does not include its source.

The connected Magic Keyboard with Touch ID (2024, USB `05ac:0321`) uses three interfaces:

| Interface | Role | Pipe |
| --- | --- | --- |
| 0 | Device management, including battery | interrupt in, 16 bytes |
| 1 | Keyboard, consumer control, system sleep | interrupt in, 16 bytes |
| 2 | Touch ID | bulk in, 64 bytes |

Interface 1 report 1 is 10 bytes. Modifiers are byte 1, six key usages start at byte 3, byte 9 bit 0 is eject, and byte 9 bit 1 is Fn.

Touch ID only authenticates to a paired Mac. This program does not drive it. Interface 0 is not bound to the inbox HID driver, so battery is not read here. On USB, Linux treats Magic Keyboard 2015 and later as having a battery report and polls it every 60 seconds. Bluetooth entries do not. The measured 2024 descriptors are 263, 207, and 49 bytes, so the 83-byte battery rewrite does not apply to this device.

```
cargo run -- status
cargo run -- listen
cargo run -- listen --fn-mode fkeysfirst --swap-opt-cmd
cargo run -- raw --list
cargo run -- raw
```

`listen` and `raw` share one translator. Keyboard report 1 goes through the Fn map. Consumer and system reports are printed as down and up edges. The boot collection is owned by the inbox keyboard driver, so `listen` also prints its Raw Input scan codes by name. Those scan codes are not Fn-translated: the Fn bit is not in them. `raw` accepts the same Fn and swap flags. Neither command injects keys back into Windows.

`--fn-mode` accepts the Linux values:

| Value | Aliases | Behavior |
| --- | --- | --- |
| `auto` (default) | `3` | Same as `fkeyslast` on these Apple keyboards |
| `fkeyslast` | `media`, `1` | F-row is brightness and media until Fn is held. Arrows become editing keys while Fn is held |
| `fkeysfirst` | `fkeys`, `2` | F-row stays F1-F12 until Fn is held. Arrows translate only while Fn is held |
| `fkeysdisabled` | `4` | F-row never translates. Arrows still translate while Fn is held |
| `disabled` | `off`, `0` | No Fn translation, including arrows |

`--swap-opt-cmd` swaps Option and Command on both sides. `--swap-opt-cmd left` swaps only the left pair. `--swap-ctrl-cmd` swaps Control and Command. `--swap-fn-leftctrl` swaps Fn and left Control. Swaps run in that order: Fn and left Control, then the ISO grave key, then Option and Command, then Control and Command.

`--iso` always swaps grave and the non-US key. `--no-iso` never does. The default swaps only when the model has the ISO quirk and the HID country code is 13. Every Magic Keyboard from 2015 on has the quirk, and so do the aluminum wireless ISO models. The inbox keyboard driver owns the boot collection, so this program does not see the country code and the default leaves grave alone.

`fix_magic_keyboard_descriptor` rewrites two descriptor shapes: the Magic Keyboard JIS logical maximum, and the 83-byte battery collection. It is not applied to the live 2024 device.
