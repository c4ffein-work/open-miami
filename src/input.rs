use crate::math::Vec2;
use std::cell::RefCell;
use std::collections::HashSet;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;
#[cfg(target_arch = "wasm32")]
use web_sys::{KeyboardEvent, MouseEvent, WheelEvent};

thread_local! {
    static PRESSED_KEYS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static PREVIOUS_PRESSED_KEYS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static MOUSE_POSITION: RefCell<Vec2> = const { RefCell::new(Vec2::zero()) };
    static MOUSE_BUTTONS: RefCell<HashSet<u16>> = RefCell::new(HashSet::new());
    static PREVIOUS_MOUSE_BUTTONS: RefCell<HashSet<u16>> = RefCell::new(HashSet::new());
    /// Keys that went DOWN since the last frame was consumed, even if they
    /// went back up before the frame sampled them — a sub-frame tap (press +
    /// release between two rAF ticks) must still register as a press.
    static LATCHED_KEYS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// Same latch for mouse buttons.
    static LATCHED_BUTTONS: RefCell<HashSet<u16>> = RefCell::new(HashSet::new());
    /// Mouse wheel travel accumulated this frame (browser `deltaY`, +down).
    static WHEEL_DELTA: RefCell<f32> = const { RefCell::new(0.0) };
    /// Printable characters typed this frame, in order, with the browser's
    /// own case / layout (Shift-uppercase, symbols) — the raw `key` of every
    /// single-character keydown, before `normalize_key` lower-cases it for
    /// the game keys. Consumed by text fields (the level editor).
    static TYPED_TEXT: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Call this at the end of each frame to update the previous input state (used
/// for edge detection of key and mouse-button presses).
pub fn end_frame() {
    PRESSED_KEYS.with(|current| {
        PREVIOUS_PRESSED_KEYS.with(|previous| {
            *previous.borrow_mut() = current.borrow().clone();
        });
    });
    LATCHED_KEYS.with(|k| k.borrow_mut().clear());
    LATCHED_BUTTONS.with(|b| b.borrow_mut().clear());
    MOUSE_BUTTONS.with(|current| {
        PREVIOUS_MOUSE_BUTTONS.with(|previous| {
            *previous.borrow_mut() = current.borrow().clone();
        });
    });
    WHEEL_DELTA.with(|d| *d.borrow_mut() = 0.0);
    TYPED_TEXT.with(|t| t.borrow_mut().clear());
}

#[cfg(target_arch = "wasm32")]
pub fn setup_input_handlers() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("No window")?;
    let document = window.document().ok_or("No document")?;

    // Keyboard handlers
    // Single-character keys are stored lowercase: with Shift held (the camera
    // look-ahead) `w` arrives as "W", which would otherwise stop movement and
    // leave "w" stuck pressed after a keyup with Shift still down.
    fn normalize_key(key: String) -> String {
        if key.chars().count() == 1 {
            key.to_lowercase()
        } else {
            key
        }
    }

    let keydown_closure = Closure::wrap(Box::new(|event: KeyboardEvent| {
        let raw = event.key();
        if raw.chars().count() == 1 && !event.ctrl_key() && !event.meta_key() {
            TYPED_TEXT.with(|t| t.borrow_mut().push_str(&raw));
        }
        let key = normalize_key(raw);
        if !event.repeat() {
            LATCHED_KEYS.with(|keys| {
                keys.borrow_mut().insert(key.clone());
            });
        }
        PRESSED_KEYS.with(|keys| {
            keys.borrow_mut().insert(key);
        });
    }) as Box<dyn FnMut(_)>);

    let keyup_closure = Closure::wrap(Box::new(|event: KeyboardEvent| {
        let key = normalize_key(event.key());
        PRESSED_KEYS.with(|keys| {
            keys.borrow_mut().remove(&key);
        });
    }) as Box<dyn FnMut(_)>);

    document
        .add_event_listener_with_callback("keydown", keydown_closure.as_ref().unchecked_ref())?;
    document.add_event_listener_with_callback("keyup", keyup_closure.as_ref().unchecked_ref())?;

    keydown_closure.forget();
    keyup_closure.forget();

    // Mouse handlers
    let canvas = document.get_element_by_id("glcanvas").ok_or("No canvas")?;

    let mousemove_closure = Closure::wrap(Box::new(|event: MouseEvent| {
        let x = event.offset_x() as f32;
        let y = event.offset_y() as f32;
        MOUSE_POSITION.with(|pos| {
            *pos.borrow_mut() = Vec2::new(x, y);
        });
    }) as Box<dyn FnMut(_)>);

    let mousedown_closure = Closure::wrap(Box::new(|event: MouseEvent| {
        let button = event.button() as u16;
        LATCHED_BUTTONS.with(|buttons| {
            buttons.borrow_mut().insert(button);
        });
        MOUSE_BUTTONS.with(|buttons| {
            buttons.borrow_mut().insert(button);
        });
    }) as Box<dyn FnMut(_)>);

    let mouseup_closure = Closure::wrap(Box::new(|event: MouseEvent| {
        let button = event.button() as u16;
        MOUSE_BUTTONS.with(|buttons| {
            buttons.borrow_mut().remove(&button);
        });
    }) as Box<dyn FnMut(_)>);

    canvas.add_event_listener_with_callback(
        "mousemove",
        mousemove_closure.as_ref().unchecked_ref(),
    )?;
    canvas.add_event_listener_with_callback(
        "mousedown",
        mousedown_closure.as_ref().unchecked_ref(),
    )?;
    canvas.add_event_listener_with_callback("mouseup", mouseup_closure.as_ref().unchecked_ref())?;

    // Right-click is the THROW button: swallow the browser context menu on the
    // game canvas so it never pops over the action.
    let contextmenu_closure = Closure::wrap(Box::new(|event: MouseEvent| {
        event.prevent_default();
    }) as Box<dyn FnMut(_)>);
    canvas.add_event_listener_with_callback(
        "contextmenu",
        contextmenu_closure.as_ref().unchecked_ref(),
    )?;

    // Mouse wheel (the level editor's zoom); the page must not scroll.
    let wheel_closure = Closure::wrap(Box::new(|event: WheelEvent| {
        event.prevent_default();
        let d = event.delta_y() as f32;
        WHEEL_DELTA.with(|w| *w.borrow_mut() += d);
    }) as Box<dyn FnMut(_)>);
    canvas.add_event_listener_with_callback("wheel", wheel_closure.as_ref().unchecked_ref())?;

    mousemove_closure.forget();
    mousedown_closure.forget();
    mouseup_closure.forget();
    contextmenu_closure.forget();
    wheel_closure.forget();

    Ok(())
}

pub fn is_key_down(key: &str) -> bool {
    PRESSED_KEYS.with(|keys| keys.borrow().contains(key))
}

/// Check if a key was just pressed this frame (not held from previous frame)
pub fn is_key_pressed(key: &str) -> bool {
    // Edge on the sampled state, OR the latch: a tap whose keyup already
    // arrived before this frame sampled still counts once.
    let edge = PRESSED_KEYS.with(|current| {
        PREVIOUS_PRESSED_KEYS
            .with(|previous| current.borrow().contains(key) && !previous.borrow().contains(key))
    });
    edge || LATCHED_KEYS.with(|latched| {
        latched.borrow().contains(key)
            && PREVIOUS_PRESSED_KEYS.with(|previous| !previous.borrow().contains(key))
    })
}

/// Whether any key or mouse button is currently held — a user gesture, which
/// is what browsers require before audio may start (`?floor=N` sessions have
/// no menu keypress to piggyback on).
pub fn any_pressed() -> bool {
    PRESSED_KEYS.with(|k| !k.borrow().is_empty()) || MOUSE_BUTTONS.with(|b| !b.borrow().is_empty())
}

pub fn mouse_position() -> Vec2 {
    MOUSE_POSITION.with(|pos| *pos.borrow())
}

/// Mouse wheel travel this frame (browser `deltaY` units, positive = wheel
/// down / away); 0 when the wheel did not move.
pub fn wheel_delta() -> f32 {
    WHEEL_DELTA.with(|d| *d.borrow())
}

/// The printable characters typed this frame (see `TYPED_TEXT`).
pub fn typed_text() -> String {
    TYPED_TEXT.with(|t| t.borrow().clone())
}

pub fn is_mouse_button_down(button: u16) -> bool {
    MOUSE_BUTTONS.with(|buttons| buttons.borrow().contains(&button))
}

/// Check if a mouse button was just pressed this frame (edge, not held).
pub fn is_mouse_button_pressed(button: u16) -> bool {
    let edge = MOUSE_BUTTONS.with(|current| {
        PREVIOUS_MOUSE_BUTTONS.with(|previous| {
            current.borrow().contains(&button) && !previous.borrow().contains(&button)
        })
    });
    edge || LATCHED_BUTTONS.with(|latched| {
        latched.borrow().contains(&button)
            && PREVIOUS_MOUSE_BUTTONS.with(|previous| !previous.borrow().contains(&button))
    })
}

// Key constants for common keys
pub mod keys {
    pub const W: &str = "w";
    pub const A: &str = "a";
    pub const S: &str = "s";
    pub const D: &str = "d";
    pub const SPACE: &str = " ";
    pub const SHIFT: &str = "Shift";
    pub const ARROW_UP: &str = "ArrowUp";
    pub const ARROW_DOWN: &str = "ArrowDown";
    pub const ARROW_LEFT: &str = "ArrowLeft";
    pub const ARROW_RIGHT: &str = "ArrowRight";
}

// Mouse button constants
pub mod mouse_buttons {
    pub const LEFT: u16 = 0;
    pub const MIDDLE: u16 = 1;
    pub const RIGHT: u16 = 2;
}
