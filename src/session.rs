//! Fn translation and modifier swaps, in the same order Linux uses:
//! Fn with left Control, the ISO grave key, Option with Command, Control
//! with Command, then the Fn layer. A key that is already down keeps the
//! action it started with when Fn changes.

use std::collections::BTreeMap;

use crate::model::MapKind;
use crate::report::{wire_usages, KeyboardReport};

/// Linux `fnmode`. `Auto` is the default and resolves to [`FnMode::FkeysLast`]
/// for the Apple keyboards this crate knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FnMode {
    /// `0`. Skip the Fn table, including arrows.
    Disabled,
    /// `1`. F-row is brightness and media until Fn is held.
    FkeysLast,
    /// `2`. Translate the F-row and the editing keys only while Fn is held.
    FkeysFirst,
    /// `3`. Same as [`FnMode::FkeysLast`] on these keyboards.
    Auto,
    /// `4`. Leave the F-row alone. Arrows and editing keys still follow Fn.
    FkeysDisabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Iso {
    Auto,
    On,
    Off,
}

/// Linux `swap_opt_cmd`: off, both sides, or the left pair only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapOptCmd {
    Off,
    Both,
    LeftOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub fn_mode: FnMode,
    pub swap_opt_cmd: SwapOptCmd,
    pub swap_ctrl_cmd: bool,
    pub swap_fn_leftctrl: bool,
    pub iso: Iso,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            fn_mode: FnMode::Auto,
            swap_opt_cmd: SwapOptCmd::Off,
            swap_ctrl_cmd: false,
            swap_fn_leftctrl: false,
            iso: Iso::Auto,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Key(u16),
    BrightnessDown,
    BrightnessUp,
    MissionControl,
    Dashboard,
    Search,
    MicrophoneMute,
    NumLock,
    Sleep,
    TrackPrevious,
    PlayPause,
    TrackNext,
    Mute,
    VolumeDown,
    VolumeUp,
    Function,
    Eject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub action: Action,
    pub down: bool,
}

struct Xlat {
    from: u16,
    to: Action,
    /// Function-row keys. Arrow and editing keys are translated only while Fn is held.
    fkey: bool,
}

const WIRELESS: &[Xlat] = &[
    xlat(0x2A, Action::Key(0x4C), false),
    xlat(0x28, Action::Key(0x49), false),
    xlat(0x3A, Action::BrightnessDown, true),
    xlat(0x3B, Action::BrightnessUp, true),
    xlat(0x3C, Action::MissionControl, true),
    xlat(0x3D, Action::Dashboard, true),
    xlat(0x3F, Action::NumLock, true),
    xlat(0x40, Action::TrackPrevious, true),
    xlat(0x41, Action::PlayPause, true),
    xlat(0x42, Action::TrackNext, true),
    xlat(0x43, Action::Mute, true),
    xlat(0x44, Action::VolumeDown, true),
    xlat(0x45, Action::VolumeUp, true),
    xlat(0x52, Action::Key(0x4B), false),
    xlat(0x51, Action::Key(0x4E), false),
    xlat(0x50, Action::Key(0x4A), false),
    xlat(0x4F, Action::Key(0x4D), false),
];

const MAGIC_2015: &[Xlat] = &[
    xlat(0x2A, Action::Key(0x4C), false),
    xlat(0x28, Action::Key(0x49), false),
    xlat(0x3A, Action::BrightnessDown, true),
    xlat(0x3B, Action::BrightnessUp, true),
    xlat(0x3C, Action::MissionControl, true),
    xlat(0x3D, Action::Dashboard, true),
    xlat(0x40, Action::TrackPrevious, true),
    xlat(0x41, Action::PlayPause, true),
    xlat(0x42, Action::TrackNext, true),
    xlat(0x43, Action::Mute, true),
    xlat(0x44, Action::VolumeDown, true),
    xlat(0x45, Action::VolumeUp, true),
    xlat(0x52, Action::Key(0x4B), false),
    xlat(0x51, Action::Key(0x4E), false),
    xlat(0x50, Action::Key(0x4A), false),
    xlat(0x4F, Action::Key(0x4D), false),
];

const MAGIC_2021: &[Xlat] = &[
    xlat(0x2A, Action::Key(0x4C), false),
    xlat(0x28, Action::Key(0x49), false),
    xlat(0x3A, Action::BrightnessDown, true),
    xlat(0x3B, Action::BrightnessUp, true),
    xlat(0x3C, Action::MissionControl, true),
    xlat(0x3D, Action::Search, true),
    xlat(0x3E, Action::MicrophoneMute, true),
    xlat(0x3F, Action::Sleep, true),
    xlat(0x40, Action::TrackPrevious, true),
    xlat(0x41, Action::PlayPause, true),
    xlat(0x42, Action::TrackNext, true),
    xlat(0x43, Action::Mute, true),
    xlat(0x44, Action::VolumeDown, true),
    xlat(0x45, Action::VolumeUp, true),
    xlat(0x52, Action::Key(0x4B), false),
    xlat(0x51, Action::Key(0x4E), false),
    xlat(0x50, Action::Key(0x4A), false),
    xlat(0x4F, Action::Key(0x4D), false),
];

const fn xlat(from: u16, to: Action, fkey: bool) -> Xlat {
    Xlat { from, to, fkey }
}

fn table(kind: MapKind) -> &'static [Xlat] {
    match kind {
        MapKind::Wireless => WIRELESS,
        MapKind::Magic2015 => MAGIC_2015,
        MapKind::Magic2021 => MAGIC_2021,
    }
}

/// HID country code Linux calls international ISO.
const COUNTRY_ISO: u8 = 13;

pub struct Session {
    map: MapKind,
    iso: bool,
    swap_opt_cmd: SwapOptCmd,
    swap_ctrl_cmd: bool,
    swap_fn_leftctrl: bool,
    fn_mode: FnMode,
    fn_down: bool,
    eject: bool,
    held: BTreeMap<u16, Action>,
}

impl Session {
    /// `country` is the HID country code. `Auto` swaps grave only when
    /// `iso_quirk` is set and `country` is 13.
    pub fn new(map: MapKind, iso_quirk: bool, country: Option<u8>, options: Options) -> Self {
        let iso = match options.iso {
            Iso::Auto => iso_quirk && country == Some(COUNTRY_ISO),
            Iso::On => true,
            Iso::Off => false,
        };
        Self {
            map,
            iso,
            swap_opt_cmd: options.swap_opt_cmd,
            swap_ctrl_cmd: options.swap_ctrl_cmd,
            swap_fn_leftctrl: options.swap_fn_leftctrl,
            fn_mode: options.fn_mode,
            fn_down: false,
            eject: false,
            held: BTreeMap::new(),
        }
    }

    pub fn push_keyboard(&mut self, report: &KeyboardReport) -> Vec<Edge> {
        let report = self.swap_fn(report);
        let mut edges = Vec::new();
        if report.function != self.fn_down {
            self.fn_down = report.function;
            edges.push(Edge {
                action: Action::Function,
                down: self.fn_down,
            });
        }
        if report.eject != self.eject {
            self.eject = report.eject;
            edges.push(Edge {
                action: Action::Eject,
                down: self.eject,
            });
        }

        let current = wire_usages(&report);
        let released: Vec<u16> = self
            .held
            .keys()
            .copied()
            .filter(|usage| !current.contains(usage))
            .collect();
        for usage in released {
            if let Some(action) = self.held.remove(&usage) {
                edges.push(Edge {
                    action,
                    down: false,
                });
            }
        }
        for usage in current {
            if self.held.contains_key(&usage) {
                continue;
            }
            let action = self.map_usage(usage);
            self.held.insert(usage, action);
            edges.push(Edge { action, down: true });
        }
        edges
    }

    /// Physical left Control becomes the Fn state. Physical Fn becomes left Control.
    fn swap_fn(&self, report: &KeyboardReport) -> KeyboardReport {
        if !self.swap_fn_leftctrl {
            return *report;
        }
        let left_ctrl = report.modifiers & 0x01 != 0;
        let mut modifiers = report.modifiers & !0x01;
        if report.function {
            modifiers |= 0x01;
        }
        KeyboardReport {
            modifiers,
            keys: report.keys,
            eject: report.eject,
            function: left_ctrl,
        }
    }

    fn map_usage(&self, wire: u16) -> Action {
        let usage = swap_ctrl_cmd(
            self.swap_ctrl_cmd,
            swap_opt_cmd(self.swap_opt_cmd, swap_iso(self.iso, wire)),
        );
        let entry = table(self.map).iter().find(|entry| entry.from == usage);
        let Some(entry) = entry else {
            return Action::Key(usage);
        };
        if should_translate(entry.fkey, self.fn_down, self.fn_mode) {
            entry.to
        } else {
            Action::Key(usage)
        }
    }
}

fn should_translate(fkey: bool, fn_down: bool, mode: FnMode) -> bool {
    match mode {
        FnMode::Disabled => false,
        FnMode::FkeysLast | FnMode::Auto => {
            if fkey {
                !fn_down
            } else {
                fn_down
            }
        }
        FnMode::FkeysFirst => fn_down,
        FnMode::FkeysDisabled => !fkey && fn_down,
    }
}

fn swap_iso(iso: bool, usage: u16) -> u16 {
    if !iso {
        return usage;
    }
    match usage {
        0x35 => 0x64,
        0x64 => 0x35,
        other => other,
    }
}

fn swap_opt_cmd(mode: SwapOptCmd, usage: u16) -> u16 {
    let both = match mode {
        SwapOptCmd::Off => return usage,
        SwapOptCmd::Both => true,
        SwapOptCmd::LeftOnly => false,
    };
    match usage {
        0xE2 => 0xE3,
        0xE3 => 0xE2,
        0xE6 if both => 0xE7,
        0xE7 if both => 0xE6,
        other => other,
    }
}

fn swap_ctrl_cmd(enabled: bool, usage: u16) -> u16 {
    if !enabled {
        return usage;
    }
    match usage {
        0xE0 => 0xE3,
        0xE3 => 0xE0,
        0xE4 => 0xE7,
        0xE7 => 0xE4,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::parse_keyboard;

    fn session(mode: FnMode) -> Session {
        Session::new(
            MapKind::Magic2021,
            true,
            None,
            Options {
                fn_mode: mode,
                ..Options::default()
            },
        )
    }

    fn down(keys: &[u8], flags: u8) -> KeyboardReport {
        let mut bytes = [0u8; 10];
        bytes[0] = 0x01;
        for (index, key) in keys.iter().enumerate() {
            bytes[3 + index] = *key;
        }
        bytes[9] = flags;
        parse_keyboard(&bytes).unwrap()
    }

    fn press(session: &mut Session, keys: &[u8], modifiers: u8, flags: u8) -> Vec<Edge> {
        let mut bytes = [0u8; 10];
        bytes[0] = 0x01;
        bytes[1] = modifiers;
        for (index, key) in keys.iter().enumerate() {
            bytes[3 + index] = *key;
        }
        bytes[9] = flags;
        session.push_keyboard(&parse_keyboard(&bytes).unwrap())
    }

    #[test]
    fn fkeyslast_turns_f1_into_brightness_until_fn() {
        let mut session = session(FnMode::FkeysLast);
        let edges = session.push_keyboard(&down(&[0x3A], 0));
        assert_eq!(
            edges,
            vec![Edge {
                action: Action::BrightnessDown,
                down: true
            }]
        );

        let edges = session.push_keyboard(&down(&[], 0));
        assert_eq!(
            edges,
            vec![Edge {
                action: Action::BrightnessDown,
                down: false
            }]
        );

        let edges = session.push_keyboard(&down(&[0x3A], 0x02));
        assert_eq!(
            edges,
            vec![
                Edge {
                    action: Action::Function,
                    down: true
                },
                Edge {
                    action: Action::Key(0x3A),
                    down: true
                },
            ]
        );
    }

    #[test]
    fn auto_matches_fkeyslast_for_f1() {
        let mut session = session(FnMode::Auto);
        let edges = session.push_keyboard(&down(&[0x3A], 0));
        assert_eq!(edges[0].action, Action::BrightnessDown);
    }

    #[test]
    fn fn_arrow_is_home_and_a_held_key_does_not_flip_when_fn_changes() {
        let mut session = session(FnMode::FkeysLast);
        let edges = session.push_keyboard(&down(&[0x3A], 0));
        assert_eq!(edges[0].action, Action::BrightnessDown);

        let edges = session.push_keyboard(&down(&[0x3A, 0x50], 0x02));
        assert!(edges.contains(&Edge {
            action: Action::Function,
            down: true
        }));
        assert!(edges.contains(&Edge {
            action: Action::Key(0x4A),
            down: true
        }));
        assert!(!edges
            .iter()
            .any(|edge| edge.action == Action::BrightnessDown && !edge.down));

        let edges = session.push_keyboard(&down(&[], 0));
        assert!(edges.contains(&Edge {
            action: Action::BrightnessDown,
            down: false
        }));
        assert!(edges.contains(&Edge {
            action: Action::Key(0x4A),
            down: false
        }));
    }

    #[test]
    fn fkeysfirst_keeps_f1_until_fn_is_held() {
        let mut session = session(FnMode::FkeysFirst);
        let edges = session.push_keyboard(&down(&[0x3A], 0));
        assert_eq!(
            edges,
            vec![Edge {
                action: Action::Key(0x3A),
                down: true
            }]
        );
        let edges = session.push_keyboard(&down(&[], 0));
        assert_eq!(
            edges,
            vec![Edge {
                action: Action::Key(0x3A),
                down: false
            }]
        );
        let edges = session.push_keyboard(&down(&[0x3A], 0x02));
        assert_eq!(
            edges,
            vec![
                Edge {
                    action: Action::Function,
                    down: true
                },
                Edge {
                    action: Action::BrightnessDown,
                    down: true
                },
            ]
        );
    }

    #[test]
    fn fkeysdisabled_leaves_f1_and_still_maps_fn_arrow() {
        let mut fkey = session(FnMode::FkeysDisabled);
        assert_eq!(
            fkey.push_keyboard(&down(&[0x3A], 0)),
            vec![Edge {
                action: Action::Key(0x3A),
                down: true
            }]
        );
        let mut arrow = session(FnMode::FkeysDisabled);
        assert_eq!(
            arrow.push_keyboard(&down(&[0x50], 0x02)),
            vec![
                Edge {
                    action: Action::Function,
                    down: true
                },
                Edge {
                    action: Action::Key(0x4A),
                    down: true
                },
            ]
        );
    }

    #[test]
    fn disabled_leaves_fn_arrow_as_an_arrow() {
        let mut session = session(FnMode::Disabled);
        assert_eq!(
            session.push_keyboard(&down(&[0x50], 0x02)),
            vec![
                Edge {
                    action: Action::Function,
                    down: true
                },
                Edge {
                    action: Action::Key(0x50),
                    down: true
                },
            ]
        );
    }

    #[test]
    fn option_command_swap_exchanges_alt_and_gui() {
        let mut session = Session::new(
            MapKind::Magic2021,
            true,
            None,
            Options {
                swap_opt_cmd: SwapOptCmd::Both,
                ..Options::default()
            },
        );
        let edges = press(&mut session, &[], 0x04, 0);
        assert_eq!(
            edges,
            vec![Edge {
                action: Action::Key(0xE3),
                down: true
            }]
        );
    }

    #[test]
    fn left_only_option_swap_keeps_right_alt() {
        let mut session = Session::new(
            MapKind::Magic2021,
            true,
            None,
            Options {
                swap_opt_cmd: SwapOptCmd::LeftOnly,
                ..Options::default()
            },
        );
        assert_eq!(
            press(&mut session, &[], 0x40, 0),
            vec![Edge {
                action: Action::Key(0xE6),
                down: true
            }]
        );
        let mut session = Session::new(
            MapKind::Magic2021,
            true,
            None,
            Options {
                swap_opt_cmd: SwapOptCmd::LeftOnly,
                ..Options::default()
            },
        );
        assert_eq!(
            press(&mut session, &[], 0x04, 0),
            vec![Edge {
                action: Action::Key(0xE3),
                down: true
            }]
        );
    }

    #[test]
    fn option_swap_runs_before_control_swap() {
        let options = Options {
            swap_opt_cmd: SwapOptCmd::Both,
            swap_ctrl_cmd: true,
            ..Options::default()
        };
        let mut session = Session::new(MapKind::Magic2021, true, None, options);
        assert_eq!(
            press(&mut session, &[], 0x04, 0),
            vec![Edge {
                action: Action::Key(0xE0),
                down: true
            }]
        );
        let mut session = Session::new(MapKind::Magic2021, true, None, options);
        assert_eq!(
            press(&mut session, &[], 0x08, 0),
            vec![Edge {
                action: Action::Key(0xE2),
                down: true
            }]
        );
    }

    #[test]
    fn fn_leftctrl_swap_uses_control_as_fn() {
        let options = Options {
            swap_fn_leftctrl: true,
            fn_mode: FnMode::FkeysLast,
            ..Options::default()
        };
        let mut session = Session::new(MapKind::Magic2021, true, None, options);
        assert_eq!(
            press(&mut session, &[0x3A], 0x01, 0),
            vec![
                Edge {
                    action: Action::Function,
                    down: true
                },
                Edge {
                    action: Action::Key(0x3A),
                    down: true
                },
            ]
        );
        let mut session = Session::new(MapKind::Magic2021, true, None, options);
        assert_eq!(
            press(&mut session, &[], 0, 0x02),
            vec![Edge {
                action: Action::Key(0xE0),
                down: true
            }]
        );
    }

    #[test]
    fn iso_auto_needs_the_quirk_and_country_13() {
        let swapped = Session::new(MapKind::Magic2021, true, Some(13), Options::default());
        let mut swapped = swapped;
        assert_eq!(
            press(&mut swapped, &[0x35], 0, 0),
            vec![Edge {
                action: Action::Key(0x64),
                down: true
            }]
        );
        let mut swapped = Session::new(MapKind::Magic2021, true, Some(13), Options::default());
        assert_eq!(
            press(&mut swapped, &[0x64], 0, 0),
            vec![Edge {
                action: Action::Key(0x35),
                down: true
            }]
        );

        let mut us = Session::new(MapKind::Magic2021, true, Some(33), Options::default());
        assert_eq!(press(&mut us, &[0x35], 0, 0)[0].action, Action::Key(0x35));

        let mut plain = Session::new(MapKind::Magic2021, false, Some(13), Options::default());
        assert_eq!(
            press(&mut plain, &[0x35], 0, 0)[0].action,
            Action::Key(0x35)
        );

        let mut forced = Session::new(
            MapKind::Wireless,
            false,
            None,
            Options {
                iso: Iso::On,
                ..Options::default()
            },
        );
        assert_eq!(
            press(&mut forced, &[0x35], 0, 0)[0].action,
            Action::Key(0x64)
        );

        let mut off = Session::new(
            MapKind::Magic2021,
            true,
            Some(13),
            Options {
                iso: Iso::Off,
                ..Options::default()
            },
        );
        assert_eq!(press(&mut off, &[0x35], 0, 0)[0].action, Action::Key(0x35));
    }

    #[test]
    fn twenty_fifteen_leaves_f5_alone() {
        let mut session = Session::new(MapKind::Magic2015, true, None, Options::default());
        let edges = session.push_keyboard(&down(&[0x3E], 0));
        assert_eq!(
            edges,
            vec![Edge {
                action: Action::Key(0x3E),
                down: true
            }]
        );
    }
}
