//! Translated actions to Windows key events.
//!
//! Fn is already consumed by the translator, so it is not injected. Brightness,
//! mission control, dashboard, microphone mute, and eject have no inbox virtual
//! key and are left unmapped. Media, sleep, num lock, and search use virtual
//! keys. Every other key uses a set-1 scan code.

use crate::scancode::scan_from_usage;
use crate::session::Action;

pub(crate) const KEYEVENTF_EXTENDEDKEY: u32 = 0x0001;
pub(crate) const KEYEVENTF_KEYUP: u32 = 0x0002;
pub(crate) const KEYEVENTF_SCANCODE: u32 = 0x0008;

const VK_SLEEP: u16 = 0x5F;
const VK_NUMLOCK: u16 = 0x90;
const VK_BROWSER_SEARCH: u16 = 0xAA;
const VK_VOLUME_MUTE: u16 = 0xAD;
const VK_VOLUME_DOWN: u16 = 0xAE;
const VK_VOLUME_UP: u16 = 0xAF;
const VK_MEDIA_NEXT_TRACK: u16 = 0xB0;
const VK_MEDIA_PREV_TRACK: u16 = 0xB1;
const VK_MEDIA_PLAY_PAUSE: u16 = 0xB3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StrokeKind {
    Scan,
    Vk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stroke {
    pub kind: StrokeKind,
    pub code: u16,
    pub extended: bool,
    pub down: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Injection {
    Stroke(Stroke),
    Skip,
    Unmapped,
}

pub(crate) fn injection(action: Action, down: bool) -> Injection {
    match action {
        // The translator already used Fn. Injecting it would press a key the
        // report does not contain.
        Action::Function => Injection::Skip,
        Action::BrightnessDown
        | Action::BrightnessUp
        | Action::MissionControl
        | Action::Dashboard
        | Action::MicrophoneMute
        | Action::Eject => Injection::Unmapped,
        Action::Sleep => vk(VK_SLEEP, down),
        Action::NumLock => vk(VK_NUMLOCK, down),
        Action::Search => vk(VK_BROWSER_SEARCH, down),
        Action::Mute => vk(VK_VOLUME_MUTE, down),
        Action::VolumeDown => vk(VK_VOLUME_DOWN, down),
        Action::VolumeUp => vk(VK_VOLUME_UP, down),
        Action::TrackNext => vk(VK_MEDIA_NEXT_TRACK, down),
        Action::TrackPrevious => vk(VK_MEDIA_PREV_TRACK, down),
        Action::PlayPause => vk(VK_MEDIA_PLAY_PAUSE, down),
        Action::Key(usage) => match scan_from_usage(usage) {
            Some((make, extended)) => Injection::Stroke(Stroke {
                kind: StrokeKind::Scan,
                code: make,
                extended,
                down,
            }),
            None => Injection::Unmapped,
        },
    }
}

pub(crate) fn event_fields(stroke: &Stroke) -> (u16, u16, u32) {
    let mut flags = 0u32;
    let (vk, scan) = match stroke.kind {
        StrokeKind::Scan => {
            flags |= KEYEVENTF_SCANCODE;
            if stroke.extended {
                flags |= KEYEVENTF_EXTENDEDKEY;
            }
            (0, stroke.code)
        }
        StrokeKind::Vk => (stroke.code, 0),
    };
    if !stroke.down {
        flags |= KEYEVENTF_KEYUP;
    }
    (vk, scan, flags)
}

fn vk(code: u16, down: bool) -> Injection {
    Injection::Stroke(Stroke {
        kind: StrokeKind::Vk,
        code,
        extended: false,
        down,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MapKind;
    use crate::present::{Step, Translator};
    use crate::report::{parse_consumer, parse_keyboard, InputReport};
    use crate::session::{Edge, FnMode, Options};

    #[test]
    fn a_is_a_set1_scan_down() {
        let mut translator = Translator::new(MapKind::Magic2021, true, Options::default());
        let keyboard = parse_keyboard(&[0x01, 0, 0, 0x04, 0, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(
            translator.steps(&InputReport::Keyboard(keyboard)),
            vec![Step::Edge(Edge {
                action: Action::Key(0x04),
                down: true,
            })]
        );
        let Injection::Stroke(stroke) = injection(Action::Key(0x04), true) else {
            panic!("A should be a stroke");
        };
        assert_eq!(stroke.kind, StrokeKind::Scan);
        assert_eq!(stroke.code, 0x1E);
        assert!(stroke.down);
        assert!(!stroke.extended);
        assert_eq!(event_fields(&stroke), (0, 0x1E, KEYEVENTF_SCANCODE));
    }

    #[test]
    fn left_is_an_extended_scan() {
        let Injection::Stroke(stroke) = injection(Action::Key(0x50), true) else {
            panic!("left should be a stroke");
        };
        assert!(stroke.extended);
        assert_eq!(stroke.code, 0x4B);
        assert_eq!(
            event_fields(&stroke),
            (0, 0x4B, KEYEVENTF_SCANCODE | KEYEVENTF_EXTENDEDKEY)
        );
        let Injection::Stroke(up) = injection(Action::Key(0x50), false) else {
            panic!("left up");
        };
        assert_eq!(
            event_fields(&up).2,
            KEYEVENTF_SCANCODE | KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP
        );
    }

    #[test]
    fn f1_without_fn_is_brightness_and_unmapped() {
        let mut translator = Translator::new(
            MapKind::Magic2021,
            true,
            Options {
                fn_mode: FnMode::FkeysLast,
                ..Options::default()
            },
        );
        let keyboard = parse_keyboard(&[0x01, 0, 0, 0x3A, 0, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(
            translator.steps(&InputReport::Keyboard(keyboard)),
            vec![Step::Edge(Edge {
                action: Action::BrightnessDown,
                down: true,
            })]
        );
        assert!(matches!(
            injection(Action::BrightnessDown, true),
            Injection::Unmapped
        ));
    }

    #[test]
    fn volume_up_is_the_media_vk() {
        let mut translator = Translator::new(MapKind::Magic2021, true, Options::default());
        let volume = parse_consumer(&[0x03, 0xE9, 0x00]).unwrap();
        assert_eq!(
            translator.steps(&InputReport::Consumer(volume)),
            vec![Step::Edge(Edge {
                action: Action::VolumeUp,
                down: true,
            })]
        );
        let Injection::Stroke(down) = injection(Action::VolumeUp, true) else {
            panic!("volume");
        };
        assert_eq!(down.kind, StrokeKind::Vk);
        assert_eq!(event_fields(&down), (VK_VOLUME_UP, 0, 0));
        let Injection::Stroke(up) = injection(Action::VolumeUp, false) else {
            panic!("volume up");
        };
        assert_eq!(event_fields(&up), (VK_VOLUME_UP, 0, KEYEVENTF_KEYUP));
    }

    #[test]
    fn function_is_not_injected() {
        assert!(matches!(injection(Action::Function, true), Injection::Skip));
        assert!(matches!(
            injection(Action::Function, false),
            Injection::Skip
        ));
    }

    #[test]
    fn apple_keys_without_a_windows_vk_stay_unmapped() {
        for action in [
            Action::BrightnessDown,
            Action::BrightnessUp,
            Action::MissionControl,
            Action::Dashboard,
            Action::MicrophoneMute,
            Action::Eject,
        ] {
            assert!(
                matches!(injection(action, true), Injection::Unmapped),
                "{action:?}"
            );
        }
    }

    #[test]
    fn media_sleep_numlock_and_search_use_virtual_keys() {
        assert_eq!(vk_code(Action::Sleep), VK_SLEEP);
        assert_eq!(vk_code(Action::NumLock), VK_NUMLOCK);
        assert_eq!(vk_code(Action::Search), VK_BROWSER_SEARCH);
        assert_eq!(vk_code(Action::Mute), VK_VOLUME_MUTE);
        assert_eq!(vk_code(Action::VolumeDown), VK_VOLUME_DOWN);
        assert_eq!(vk_code(Action::TrackNext), VK_MEDIA_NEXT_TRACK);
        assert_eq!(vk_code(Action::TrackPrevious), VK_MEDIA_PREV_TRACK);
        assert_eq!(vk_code(Action::PlayPause), VK_MEDIA_PLAY_PAUSE);
    }

    fn vk_code(action: Action) -> u16 {
        let Injection::Stroke(stroke) = injection(action, true) else {
            panic!("{action:?}");
        };
        assert_eq!(stroke.kind, StrokeKind::Vk);
        stroke.code
    }
}
