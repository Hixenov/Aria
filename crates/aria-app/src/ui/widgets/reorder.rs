//! Reordering a list by dragging: the grip a row is dragged by, and the row a drag lands on.

use eframe::egui::{DragAndDrop, Id, InnerResponse, Rect, Sense, Stroke, Ui, vec2};

use crate::ui::theme::Palette;

/// Width of the grip, and the radius of its dots.
pub const GRIP: [f32; 2] = [10.0, 1.2];
/// Width of the line that shows where a dragged row would land.
const DROP_LINE: f32 = 2.0;

/// What is being dragged: the key of the row it came from.
struct Dragged<K>(K);

/// Makes `content` something a row can be dragged by. `key` names the row.
pub fn drag_source<K, R>(
    ui: &mut Ui,
    id: Id,
    key: K,
    content: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R>
where
    K: Send + Sync + 'static,
{
    ui.dnd_drag_source(id, Dragged(key), content)
}

/// Two columns of three dots, `height` tall.
pub fn grip(ui: &mut Ui, p: &Palette, height: f32) {
    let [width, dot] = GRIP;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    for column in [-1.0, 1.0] {
        for line in [-1.0, 0.0, 1.0] {
            let at = rect.center() + vec2(column * width / 4.0, line * 5.0);
            ui.painter().circle_filled(at, dot, p.text_muted);
        }
    }
}

/// Treats the row at `rect`, named `key`, as a drop target. While a row is dragged over it,
/// draws where that row would land; when one is released on it, returns the key it came from.
/// Releasing a row on itself returns nothing.
pub fn drop_target<K>(ui: &Ui, p: &Palette, rect: Rect, key: K) -> Option<K>
where
    K: Copy + PartialEq + PartialOrd + Send + Sync + 'static,
{
    // Where the pointer is, not `Response::contains_pointer`: that is false while a widget
    // inside the row covers the pointer.
    let over = ui
        .input(|input| input.pointer.interact_pos())
        .is_some_and(|pos| rect.intersect(ui.clip_rect()).contains(pos));
    if !over {
        return None;
    }
    if ui.input(|input| input.pointer.any_released()) {
        let from = DragAndDrop::take_payload::<Dragged<K>>(ui.ctx())?.0;
        return (from != key).then_some(from);
    }
    if let Some(from) = DragAndDrop::payload::<Dragged<K>>(ui.ctx())
        && from.0 != key
    {
        // Moving up lands above this row, moving down below it.
        let y = if key < from.0 {
            rect.top()
        } else {
            rect.bottom()
        };
        ui.painter()
            .hline(rect.x_range(), y, Stroke::new(DROP_LINE, p.accent));
    }
    None
}
