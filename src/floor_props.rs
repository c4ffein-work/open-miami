//! Placed floor props (`FloorDef::props`, `levels/*.json` → `props`): drawn
//! in WORLD space over the floor tiles and walls, under the actors, with the
//! prop's own saved pixel-art settings (`props/props.json`), rotated by the
//! placement's `rot`. Decoration only — no collision (phase 1). Shared by
//! the game (`update_game`) and the native level editor.

use crate::graphics::Graphics;
use crate::math::Vec2;
use crate::props::{draw_prop_ex, prop_px, PropDrawOpts, PROP_COUNT};
use crate::scenario::PropPlacement;

/// Draw one placed prop, centred on `(p.x, p.y)`, `p.size` world units
/// across, turned by `p.rot` degrees (clockwise, +y down). `time` is the
/// continuous animation clock (seconds).
pub fn draw_placed_prop(g: &Graphics, p: &PropPlacement, time: f32) {
    let kind = p.kind % PROP_COUNT;
    g.save();
    g.translate(p.x, p.y);
    if p.rot != 0.0 {
        g.rotate(p.rot.to_radians());
    }
    draw_prop_ex(
        g,
        kind,
        Vec2::zero(),
        p.size,
        time,
        prop_px(kind),
        &PropDrawOpts::saved(kind),
    );
    g.restore();
}

/// Draw every placed prop of a floor (call between the walls and the actors,
/// with the camera transform applied).
pub fn render_floor_props(g: &Graphics, props: &[PropPlacement], time: f32) {
    for p in props {
        draw_placed_prop(g, p, time);
    }
}
