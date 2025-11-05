use std::cell::RefCell;
use std::collections::HashSet;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;
#[cfg(target_arch = "wasm32")]
use web_sys::{KeyboardEvent, MouseEvent};
use crate::math::Vec2;

thread_local! {
    static PRESSED_KEYS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static MOUSE_POSITION: RefCell<Vec2> = RefCell::new(Vec2::zero());
    static MOUSE_BUTTONS: RefCell<HashSet<u16>> = RefCell::new(HashSet::new());
}

#[cfg(target_arch = "wasm32")]
pub fn setup_input_handlers() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("No window")?;
    let document = window.document().ok_or("No document")?;

    // Keyboard handlers
    let keydown_closure = Closure::wrap(Box::new(|event: KeyboardEvent| {
        let key = event.key();
        PRESSED_KEYS.with(|keys| {
            keys.borrow_mut().insert(key);
        });
    }) as Box<dyn FnMut(_)>);

    let keyup_closure = Closure::wrap(Box::new(|event: KeyboardEvent| {
        let key = event.key();
        PRESSED_KEYS.with(|keys| {
            keys.borrow_mut().remove(&key);
        });
    }) as Box<dyn FnMut(_)>);

    document.add_event_listener_with_callback("keydown", keydown_closure.as_ref().unchecked_ref())?;
    document.add_event_listener_with_callback("keyup", keyup_closure.as_ref().unchecked_ref())?;

    keydown_closure.forget();
    keyup_closure.forget();

    // Mouse handlers
    let canvas = document.get_element_by_id("canvas").ok_or("No canvas")?;

    let mousemove_closure = Closure::wrap(Box::new(|event: MouseEvent| {
        let x = event.offset_x() as f32;
        let y = event.offset_y() as f32;
        MOUSE_POSITION.with(|pos| {
            *pos.borrow_mut() = Vec2::new(x, y);
        });
    }) as Box<dyn FnMut(_)>);

    let mousedown_closure = Closure::wrap(Box::new(|event: MouseEvent| {
        let button = event.button();
        MOUSE_BUTTONS.with(|buttons| {
            buttons.borrow_mut().insert(button);
        });
    }) as Box<dyn FnMut(_)>);

    let mouseup_closure = Closure::wrap(Box::new(|event: MouseEvent| {
        let button = event.button();
        MOUSE_BUTTONS.with(|buttons| {
            buttons.borrow_mut().remove(&button);
        });
    }) as Box<dyn FnMut(_)>);

    canvas.add_event_listener_with_callback("mousemove", mousemove_closure.as_ref().unchecked_ref())?;
    canvas.add_event_listener_with_callback("mousedown", mousedown_closure.as_ref().unchecked_ref())?;
    canvas.add_event_listener_with_callback("mouseup", mouseup_closure.as_ref().unchecked_ref())?;

    mousemove_closure.forget();
    mousedown_closure.forget();
    mouseup_closure.forget();

    Ok(())
}

pub fn is_key_down(key: &str) -> bool {
    PRESSED_KEYS.with(|keys| keys.borrow().contains(key))
}

pub fn mouse_position() -> Vec2 {
    MOUSE_POSITION.with(|pos| *pos.borrow())
}

pub fn is_mouse_button_down(button: u16) -> bool {
    MOUSE_BUTTONS.with(|buttons| buttons.borrow().contains(&button))
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
