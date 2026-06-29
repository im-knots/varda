//! winit → [`HtmlInputEvent`] translation for interactive HTML decks.
//!
//! All conversion from winit input types into the engine's `Send`
//! [`HtmlInputEvent`] happens here so winit never reaches `internal/html`.
//! Cursor positions are mapped from window logical coordinates to WebView
//! **device** pixels (fixed 1:1, see `/spec/html-source.md` §4).
//!
//! Key/Code/NamedKey names follow the W3C UI Events spec in both winit and
//! `keyboard_types`, so most map via `FromStr` on the variant name with a few
//! documented special cases (winit `Super` ↔ W3C `Meta`). These are pure,
//! stateless helpers; IME composition start/end tracking lives in the caller.

use std::str::FromStr;

use keyboard_types::{
    Code, CompositionEvent, CompositionState, Key, KeyState, KeyboardEvent, Location, Modifiers,
    NamedKey,
};
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta};
use winit::keyboard::{
    Key as WKey, KeyLocation, ModifiersState, NamedKey as WNamedKey, PhysicalKey,
};

use crate::html::HtmlInputEvent;

/// Lines→pixels factor for `MouseScrollDelta::LineDelta` (typical UA line height).
const LINE_HEIGHT_PX: f64 = 16.0;

/// Map window logical cursor coords → WebView device-pixel point, clamped to the
/// WebView size. `scale` is the window scale factor (device px per logical px).
pub fn to_device_point(logical: (f64, f64), scale: f64, size: (u32, u32)) -> (f32, f32) {
    let max_x = size.0.saturating_sub(1) as f64;
    let max_y = size.1.saturating_sub(1) as f64;
    let x = (logical.0 * scale).clamp(0.0, max_x);
    let y = (logical.1 * scale).clamp(0.0, max_y);
    (x as f32, y as f32)
}

/// DOM button index for a winit mouse button (0=left, 1=middle, 2=right, …).
pub fn mouse_button_index(button: MouseButton) -> u16 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::Back => 3,
        MouseButton::Forward => 4,
        MouseButton::Other(n) => n,
    }
}

/// A pointer-move event at a WebView device-pixel point.
pub fn mouse_move(point: (f32, f32)) -> HtmlInputEvent {
    HtmlInputEvent::MouseMove {
        x: point.0,
        y: point.1,
    }
}

/// A mouse button press/release at a WebView device-pixel point.
pub fn mouse_button(point: (f32, f32), button: MouseButton, state: ElementState) -> HtmlInputEvent {
    HtmlInputEvent::MouseButton {
        x: point.0,
        y: point.1,
        button: mouse_button_index(button),
        pressed: state == ElementState::Pressed,
    }
}

/// A wheel event in device pixels. winit and Servo agree on sign (positive = the
/// view scrolls up/left, revealing earlier content), so deltas pass through.
/// `PixelDelta` is already physical px; `LineDelta` is scaled by line height.
pub fn wheel(point: (f32, f32), delta: MouseScrollDelta) -> HtmlInputEvent {
    let (dx, dy) = match delta {
        MouseScrollDelta::LineDelta(x, y) => (x as f64 * LINE_HEIGHT_PX, y as f64 * LINE_HEIGHT_PX),
        MouseScrollDelta::PixelDelta(p) => (p.x, p.y),
    };
    HtmlInputEvent::Wheel {
        x: point.0,
        y: point.1,
        dx,
        dy,
    }
}

/// Translate a winit key event (+ current modifiers) into a keyboard event.
pub fn keyboard(event: &KeyEvent, mods: ModifiersState) -> HtmlInputEvent {
    let state = match event.state {
        ElementState::Pressed => KeyState::Down,
        ElementState::Released => KeyState::Up,
    };
    let kt = KeyboardEvent {
        state,
        key: logical_key(&event.logical_key, event.text.as_deref()),
        code: physical_code(event.physical_key),
        location: key_location(event.location),
        modifiers: modifiers(mods),
        repeat: event.repeat,
        is_composing: false,
    };
    HtmlInputEvent::Key(kt)
}

/// An IME preedit (composition) update. `start` marks the first event of a
/// session (`compositionstart`); subsequent events are `compositionupdate`.
pub fn ime_preedit(text: String, start: bool) -> HtmlInputEvent {
    let state = if start {
        CompositionState::Start
    } else {
        CompositionState::Update
    };
    HtmlInputEvent::Ime(CompositionEvent { state, data: text })
}

/// An IME commit (`compositionend`) — the composed text is inserted.
pub fn ime_commit(text: String) -> HtmlInputEvent {
    HtmlInputEvent::Ime(CompositionEvent {
        state: CompositionState::End,
        data: text,
    })
}

fn logical_key(key: &WKey, text: Option<&str>) -> Key {
    match key {
        WKey::Character(s) => Key::Character(s.as_str().to_string()),
        WKey::Named(WNamedKey::Space) => Key::Character(" ".to_string()),
        WKey::Named(nk) => named_key(*nk),
        WKey::Dead(_) => match text {
            Some(t) if !t.is_empty() => Key::Character(t.to_string()),
            _ => Key::Named(NamedKey::Unidentified),
        },
        WKey::Unidentified(_) => Key::Named(NamedKey::Unidentified),
    }
}

fn named_key(nk: WNamedKey) -> Key {
    // winit `Super` is the OS/Cmd/Win key; W3C / keyboard_types call it `Meta`.
    if matches!(nk, WNamedKey::Super) {
        return Key::Named(NamedKey::Meta);
    }
    Key::Named(NamedKey::from_str(&format!("{nk:?}")).unwrap_or(NamedKey::Unidentified))
}

fn physical_code(pk: PhysicalKey) -> Code {
    match pk {
        PhysicalKey::Code(kc) => {
            // winit `SuperLeft`/`SuperRight` ↔ W3C `MetaLeft`/`MetaRight`.
            let name = match format!("{kc:?}").as_str() {
                "SuperLeft" => "MetaLeft".to_string(),
                "SuperRight" => "MetaRight".to_string(),
                other => other.to_string(),
            };
            Code::from_str(&name).unwrap_or(Code::Unidentified)
        }
        PhysicalKey::Unidentified(_) => Code::Unidentified,
    }
}

fn key_location(loc: KeyLocation) -> Location {
    match loc {
        KeyLocation::Standard => Location::Standard,
        KeyLocation::Left => Location::Left,
        KeyLocation::Right => Location::Right,
        KeyLocation::Numpad => Location::Numpad,
    }
}

fn modifiers(m: ModifiersState) -> Modifiers {
    let mut out = Modifiers::empty();
    if m.shift_key() {
        out |= Modifiers::SHIFT;
    }
    if m.control_key() {
        out |= Modifiers::CONTROL;
    }
    if m.alt_key() {
        out |= Modifiers::ALT;
    }
    if m.super_key() {
        out |= Modifiers::META;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::dpi::PhysicalPosition;
    use winit::keyboard::KeyCode;

    #[test]
    fn device_point_scales_and_clamps() {
        // 1:1 at scale 1.0 within bounds.
        assert_eq!(to_device_point((10.0, 20.0), 1.0, (320, 240)), (10.0, 20.0));
        // Scale factor applied (HiDPI).
        assert_eq!(to_device_point((10.0, 20.0), 2.0, (640, 480)), (20.0, 40.0));
        // Clamped to size-1 and to >= 0.
        assert_eq!(
            to_device_point((9999.0, -5.0), 1.0, (320, 240)),
            (319.0, 0.0)
        );
    }

    #[test]
    fn mouse_button_indices() {
        assert_eq!(mouse_button_index(MouseButton::Left), 0);
        assert_eq!(mouse_button_index(MouseButton::Middle), 1);
        assert_eq!(mouse_button_index(MouseButton::Right), 2);
        assert_eq!(mouse_button_index(MouseButton::Other(7)), 7);
    }

    #[test]
    fn mouse_button_press_release() {
        match mouse_button((1.0, 2.0), MouseButton::Left, ElementState::Pressed) {
            HtmlInputEvent::MouseButton {
                button, pressed, ..
            } => {
                assert_eq!(button, 0);
                assert!(pressed);
            }
            other => panic!("unexpected: {other:?}"),
        }
        match mouse_button((1.0, 2.0), MouseButton::Right, ElementState::Released) {
            HtmlInputEvent::MouseButton {
                button, pressed, ..
            } => {
                assert_eq!(button, 2);
                assert!(!pressed);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn wheel_line_vs_pixel() {
        match wheel((0.0, 0.0), MouseScrollDelta::LineDelta(0.0, 1.0)) {
            HtmlInputEvent::Wheel { dy, .. } => assert_eq!(dy, LINE_HEIGHT_PX),
            other => panic!("unexpected: {other:?}"),
        }
        match wheel(
            (0.0, 0.0),
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(3.0, 4.0)),
        ) {
            HtmlInputEvent::Wheel { dx, dy, .. } => {
                assert_eq!(dx, 3.0);
                assert_eq!(dy, 4.0);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn logical_key_character_and_space() {
        assert_eq!(
            logical_key(&WKey::Character("a".into()), Some("a")),
            Key::Character("a".to_string())
        );
        assert_eq!(
            logical_key(&WKey::Named(WNamedKey::Space), Some(" ")),
            Key::Character(" ".to_string())
        );
    }

    #[test]
    fn named_key_and_super_to_meta() {
        assert_eq!(named_key(WNamedKey::Enter), Key::Named(NamedKey::Enter));
        assert_eq!(named_key(WNamedKey::Escape), Key::Named(NamedKey::Escape));
        assert_eq!(
            named_key(WNamedKey::ArrowLeft),
            Key::Named(NamedKey::ArrowLeft)
        );
        // winit Super maps to W3C Meta.
        assert_eq!(named_key(WNamedKey::Super), Key::Named(NamedKey::Meta));
    }

    #[test]
    fn physical_code_and_super_left_to_meta_left() {
        assert_eq!(physical_code(PhysicalKey::Code(KeyCode::KeyA)), Code::KeyA);
        assert_eq!(
            physical_code(PhysicalKey::Code(KeyCode::Enter)),
            Code::Enter
        );
        assert_eq!(
            physical_code(PhysicalKey::Code(KeyCode::SuperLeft)),
            Code::MetaLeft
        );
    }

    #[test]
    fn location_and_modifiers() {
        assert_eq!(key_location(KeyLocation::Numpad), Location::Numpad);
        assert_eq!(key_location(KeyLocation::Left), Location::Left);
        let m = modifiers(ModifiersState::SHIFT | ModifiersState::SUPER);
        assert!(m.contains(Modifiers::SHIFT));
        assert!(m.contains(Modifiers::META));
        assert!(!m.contains(Modifiers::CONTROL));
    }

    #[test]
    fn ime_helpers() {
        match ime_preedit("ni".to_string(), true) {
            HtmlInputEvent::Ime(c) => {
                assert_eq!(c.state, CompositionState::Start);
                assert_eq!(c.data, "ni");
            }
            other => panic!("unexpected: {other:?}"),
        }
        match ime_commit("你".to_string()) {
            HtmlInputEvent::Ime(c) => {
                assert_eq!(c.state, CompositionState::End);
                assert_eq!(c.data, "你");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
