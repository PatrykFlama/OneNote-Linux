mod drawing;
mod history;
mod rendering;

use crate::project::{Asset, CanvasPoint, EditorBlock, EditorPage, EditorStroke};
use drawing::{
    current_touch_force, erase_at, ink_at, ink_block_from_strokes, pen_palette, sampled_pen_width,
    shape_is_large_enough, shape_strokes, smooth_stroke,
};
use eframe::egui;
use history::{PageSnapshot, commit_pending_history, push_history};
use rendering::{
    expand_canvas, layout_rect, paint_ink, paint_page_background, paint_stroke,
    render_positioned_block,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CanvasTool {
    Select,
    Pen,
    Highlighter,
    Line,
    Arrow,
    Rectangle,
    Ellipse,
    Eraser,
}

#[derive(Default)]
struct ToolbarOutcome {
    changed: bool,
    undo: bool,
    redo: bool,
    history: Option<PageSnapshot>,
}

pub struct CanvasEditor {
    tool: CanvasTool,
    scene_rect: egui::Rect,
    scene_page_id: Option<u64>,
    selected_block: Option<u64>,
    active_strokes: Vec<EditorStroke>,
    gesture_start: Option<CanvasPoint>,
    pen_color: egui::Color32,
    pen_width: f32,
    smoothing: f32,
    dynamic_width: bool,
    eraser_size: f32,
    undo_stack: Vec<PageSnapshot>,
    redo_stack: Vec<PageSnapshot>,
    pending_history: Option<PageSnapshot>,
}

impl Default for CanvasEditor {
    fn default() -> Self {
        Self {
            tool: CanvasTool::Select,
            scene_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0)),
            scene_page_id: None,
            selected_block: None,
            active_strokes: Vec::new(),
            gesture_start: None,
            pen_color: egui::Color32::from_rgb(35, 35, 38),
            pen_width: 2.5,
            smoothing: 0.55,
            dynamic_width: true,
            eraser_size: 24.0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            pending_history: None,
        }
    }
}

impl CanvasEditor {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        page: &mut EditorPage,
        assets: &[Asset],
        next_id: &mut u64,
        error: &mut Option<String>,
    ) -> bool {
        if self.scene_page_id != Some(page.id) {
            self.scene_page_id = Some(page.id);
            self.scene_rect =
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
            self.selected_block = None;
            self.active_strokes.clear();
            self.gesture_start = None;
            self.undo_stack.clear();
            self.redo_stack.clear();
            self.pending_history = None;
        }

        let keyboard_redo = ui.input_mut(|input| {
            input.consume_key(
                egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
                egui::Key::Z,
            ) || input.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)
        });
        let keyboard_undo =
            ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z));

        let toolbar = self.toolbar(ui, page);
        let mut changed = toolbar.changed;
        if toolbar.changed
            && let Some(snapshot) = toolbar.history
        {
            self.record_history(snapshot);
        }
        let history_changed = if toolbar.redo || keyboard_redo {
            self.redo(page)
        } else if toolbar.undo || keyboard_undo {
            self.undo(page)
        } else {
            false
        };
        changed |= history_changed;
        ui.separator();

        let scene = egui::Scene::new()
            .zoom_range(0.2..=4.0)
            .max_inner_size(egui::vec2(page.canvas_width, page.canvas_height))
            .drag_pan_buttons(egui::DragPanButtons::MIDDLE | egui::DragPanButtons::SECONDARY);

        let tool = self.tool;
        let active_pen = self.active_pen();
        let smoothing = self.smoothing;
        let dynamic_width = self.dynamic_width;
        let eraser_size = self.eraser_size;
        let touch_force = current_touch_force(ui);
        let pointer_speed = ui.input(|input| input.pointer.velocity().length());
        let shift_down = ui.input(|input| input.modifiers.shift);
        let selected_block = &mut self.selected_block;
        let active_strokes = &mut self.active_strokes;
        let gesture_start = &mut self.gesture_start;
        let scene_rect = &mut self.scene_rect;
        let pending_history = &mut self.pending_history;
        let undo_stack = &mut self.undo_stack;
        let redo_stack = &mut self.redo_stack;

        scene.show(ui, scene_rect, |ui| {
            let canvas_rect = egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(page.canvas_width, page.canvas_height),
            );
            ui.set_min_size(canvas_rect.size());
            paint_page_background(ui.painter(), canvas_rect);

            for block in &mut page.blocks {
                if !matches!(block, EditorBlock::Ink { .. }) {
                    changed |=
                        render_positioned_block(ui, block, assets, tool, selected_block, error);
                }
            }

            for block in &mut page.blocks {
                if let EditorBlock::Ink {
                    id,
                    strokes,
                    layout,
                    ..
                } = block
                {
                    paint_ink(ui.painter(), *layout, strokes);
                    if tool == CanvasTool::Select {
                        let response = ui.interact(
                            layout_rect(*layout).expand(6.0),
                            ui.id().with(("ink", *id)),
                            egui::Sense::click_and_drag(),
                        );
                        if response.clicked() {
                            *selected_block = Some(*id);
                        }
                        if response.dragged() {
                            let delta = response.drag_delta();
                            layout.x += delta.x;
                            layout.y += delta.y;
                            changed = true;
                        }
                    }
                }
            }

            for stroke in active_strokes.iter() {
                paint_stroke(ui.painter(), egui::Pos2::ZERO, stroke);
            }

            if tool == CanvasTool::Select {
                if let Some(selected) = *selected_block
                    && let Some(block) = page.blocks.iter().find(|block| block.id() == selected)
                {
                    ui.painter().rect_stroke(
                        layout_rect(block.layout()).expand(4.0),
                        2.0,
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(91, 134, 229)),
                        egui::StrokeKind::Outside,
                    );
                }
            } else {
                let response = ui.interact(
                    canvas_rect,
                    ui.id().with("ink_overlay"),
                    egui::Sense::click_and_drag(),
                );
                let pointer = response.interact_pointer_pos();
                match tool {
                    CanvasTool::Pen | CanvasTool::Highlighter => {
                        if response.drag_started_by(egui::PointerButton::Primary)
                            && let Some(position) = pointer
                        {
                            pending_history.get_or_insert_with(|| PageSnapshot::capture(page));
                            let (color, width) = active_pen;
                            let sampled_width = sampled_pen_width(
                                width,
                                touch_force,
                                pointer_speed,
                                dynamic_width && tool == CanvasTool::Pen,
                            );
                            *active_strokes = vec![EditorStroke {
                                points: vec![CanvasPoint {
                                    x: position.x,
                                    y: position.y,
                                }],
                                color,
                                width,
                                point_widths: if dynamic_width && tool == CanvasTool::Pen {
                                    vec![sampled_width]
                                } else {
                                    Vec::new()
                                },
                            }];
                        }
                        if response.dragged_by(egui::PointerButton::Primary)
                            && let Some(position) = pointer
                            && let Some(stroke) = active_strokes.first_mut()
                        {
                            let should_add = stroke.points.last().is_none_or(|previous| {
                                let delta =
                                    egui::vec2(position.x - previous.x, position.y - previous.y);
                                delta.length_sq() > 0.6
                            });
                            if should_add {
                                stroke.points.push(CanvasPoint {
                                    x: position.x,
                                    y: position.y,
                                });
                                if !stroke.point_widths.is_empty() {
                                    stroke.point_widths.push(sampled_pen_width(
                                        stroke.width,
                                        touch_force,
                                        pointer_speed,
                                        true,
                                    ));
                                }
                            }
                        }
                        if response.drag_stopped_by(egui::PointerButton::Primary) {
                            let mut strokes = std::mem::take(active_strokes);
                            if let Some(stroke) = strokes.first_mut() {
                                smooth_stroke(stroke, smoothing);
                            }
                            if strokes
                                .first()
                                .is_some_and(|stroke| stroke.points.len() >= 2)
                            {
                                let id = take_id(next_id);
                                page.blocks.push(ink_block_from_strokes(id, strokes));
                                commit_pending_history(pending_history, undo_stack, redo_stack);
                                changed = true;
                            } else {
                                pending_history.take();
                            }
                        }
                    }
                    CanvasTool::Line
                    | CanvasTool::Arrow
                    | CanvasTool::Rectangle
                    | CanvasTool::Ellipse => {
                        if response.drag_started_by(egui::PointerButton::Primary)
                            && let Some(position) = pointer
                        {
                            pending_history.get_or_insert_with(|| PageSnapshot::capture(page));
                            let start = CanvasPoint {
                                x: position.x,
                                y: position.y,
                            };
                            *gesture_start = Some(start);
                            let (color, width) = active_pen;
                            *active_strokes =
                                shape_strokes(tool, start, start, color, width, shift_down);
                        }
                        if response.dragged_by(egui::PointerButton::Primary)
                            && let (Some(start), Some(position)) = (*gesture_start, pointer)
                        {
                            let (color, width) = active_pen;
                            let end = CanvasPoint {
                                x: position.x,
                                y: position.y,
                            };
                            *active_strokes =
                                shape_strokes(tool, start, end, color, width, shift_down);
                        }
                        if response.drag_stopped_by(egui::PointerButton::Primary) {
                            let strokes = std::mem::take(active_strokes);
                            *gesture_start = None;
                            if shape_is_large_enough(&strokes) {
                                let id = take_id(next_id);
                                page.blocks.push(ink_block_from_strokes(id, strokes));
                                commit_pending_history(pending_history, undo_stack, redo_stack);
                                changed = true;
                            } else {
                                pending_history.take();
                            }
                        }
                    }
                    CanvasTool::Eraser => {
                        if (response.dragged_by(egui::PointerButton::Primary)
                            || response.clicked_by(egui::PointerButton::Primary))
                            && let Some(position) = pointer
                        {
                            let radius = eraser_size * 0.5;
                            if ink_at(&page.blocks, position, radius) {
                                pending_history.get_or_insert_with(|| PageSnapshot::capture(page));
                                if erase_at(&mut page.blocks, position, radius) {
                                    changed = true;
                                }
                            }
                            ui.painter().circle_stroke(
                                position,
                                radius,
                                egui::Stroke::new(1.0, egui::Color32::from_gray(90)),
                            );
                        }
                        if response.drag_stopped_by(egui::PointerButton::Primary)
                            || response.clicked_by(egui::PointerButton::Primary)
                        {
                            commit_pending_history(pending_history, undo_stack, redo_stack);
                        }
                    }
                    CanvasTool::Select => {}
                }
            }

            if changed {
                expand_canvas(page);
            }
        });

        changed
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, page: &mut EditorPage) -> ToolbarOutcome {
        let mut outcome = ToolbarOutcome::default();
        ui.horizontal_wrapped(|ui| {
            outcome.undo = ui
                .add_enabled(!self.undo_stack.is_empty(), egui::Button::new("Undo"))
                .on_hover_text("Ctrl+Z")
                .clicked();
            outcome.redo = ui
                .add_enabled(!self.redo_stack.is_empty(), egui::Button::new("Redo"))
                .on_hover_text("Ctrl+Shift+Z or Ctrl+Y")
                .clicked();
            ui.separator();
            ui.selectable_value(&mut self.tool, CanvasTool::Select, "Select");
            ui.selectable_value(&mut self.tool, CanvasTool::Pen, "Pen");
            ui.selectable_value(&mut self.tool, CanvasTool::Highlighter, "Highlighter");
            ui.selectable_value(&mut self.tool, CanvasTool::Line, "Line");
            ui.selectable_value(&mut self.tool, CanvasTool::Arrow, "Arrow");
            ui.selectable_value(&mut self.tool, CanvasTool::Rectangle, "Rectangle");
            ui.selectable_value(&mut self.tool, CanvasTool::Ellipse, "Ellipse");
            ui.selectable_value(&mut self.tool, CanvasTool::Eraser, "Eraser");
            ui.separator();
            for color in pen_palette() {
                let selected = self.pen_color == color;
                let button = egui::Button::new("")
                    .min_size(egui::vec2(18.0, 18.0))
                    .fill(color)
                    .stroke(if selected {
                        egui::Stroke::new(2.0, egui::Color32::WHITE)
                    } else {
                        egui::Stroke::new(1.0, egui::Color32::from_gray(100))
                    });
                if ui.add(button).clicked() {
                    self.pen_color = color;
                }
            }
            ui.color_edit_button_srgba(&mut self.pen_color)
                .on_hover_text("Custom color");
            ui.add(egui::Slider::new(&mut self.pen_width, 1.0..=16.0).text("Width"));
            for (label, width) in [("1.5", 1.5), ("2.5", 2.5), ("4", 4.0), ("8", 8.0)] {
                if ui.small_button(label).clicked() {
                    self.pen_width = width;
                }
            }
            if matches!(self.tool, CanvasTool::Pen | CanvasTool::Highlighter) {
                ui.add(egui::Slider::new(&mut self.smoothing, 0.0..=1.0).text("Smooth"));
            }
            if self.tool == CanvasTool::Pen {
                ui.checkbox(&mut self.dynamic_width, "Dynamics")
                    .on_hover_text("Use pressure when available; otherwise vary width with speed");
            }
            if self.tool == CanvasTool::Eraser {
                ui.add(egui::Slider::new(&mut self.eraser_size, 6.0..=80.0).text("Eraser"));
            }
            ui.separator();
            if ui.button("Reset view").clicked() {
                self.scene_rect =
                    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
            }
            if ui
                .add_enabled(
                    self.selected_block.is_some(),
                    egui::Button::new("Delete selected"),
                )
                .clicked()
                && let Some(selected) = self.selected_block.take()
            {
                outcome
                    .history
                    .get_or_insert_with(|| PageSnapshot::capture(page));
                let before = page.blocks.len();
                page.blocks.retain(|block| block.id() != selected);
                outcome.changed |= page.blocks.len() != before;
            }
            let selected_ink = self.selected_block.and_then(|selected| {
                let EditorBlock::Ink { strokes, .. } =
                    page.blocks.iter().find(|block| block.id() == selected)?
                else {
                    return None;
                };
                let first = strokes.first()?;
                Some((selected, first.color, first.width))
            });
            if let Some((selected, selected_color, selected_width)) = selected_ink {
                ui.separator();
                let mut color = egui::Color32::from_rgba_unmultiplied(
                    selected_color[0],
                    selected_color[1],
                    selected_color[2],
                    selected_color[3],
                );
                let mut width = selected_width;
                let color_changed = ui.color_edit_button_srgba(&mut color).changed();
                let width_changed = ui
                    .add(egui::Slider::new(&mut width, 1.0..=24.0).text("Selected ink"))
                    .changed();
                if color_changed || width_changed {
                    outcome
                        .history
                        .get_or_insert_with(|| PageSnapshot::capture(page));
                    if let Some(EditorBlock::Ink { strokes, .. }) =
                        page.blocks.iter_mut().find(|block| block.id() == selected)
                    {
                        for stroke in strokes {
                            if color_changed {
                                stroke.color = [color.r(), color.g(), color.b(), color.a()];
                            }
                            if width_changed {
                                let ratio = if stroke.width > f32::EPSILON {
                                    width / stroke.width
                                } else {
                                    1.0
                                };
                                for point_width in &mut stroke.point_widths {
                                    *point_width *= ratio;
                                }
                                stroke.width = width;
                            }
                        }
                    }
                    outcome.changed = true;
                }
            }
            ui.weak(
                "Shift constrains shapes; middle/right drag or scroll pans; Ctrl+wheel/pinch zooms",
            );
        });
        outcome
    }

    fn active_pen(&self) -> ([u8; 4], f32) {
        match self.tool {
            CanvasTool::Highlighter => {
                let color = self.pen_color;
                (
                    [color.r(), color.g(), color.b(), 80],
                    self.pen_width.max(8.0),
                )
            }
            _ => {
                let color = self.pen_color;
                ([color.r(), color.g(), color.b(), color.a()], self.pen_width)
            }
        }
    }

    fn record_history(&mut self, snapshot: PageSnapshot) {
        push_history(&mut self.undo_stack, snapshot);
        self.redo_stack.clear();
    }

    fn undo(&mut self, page: &mut EditorPage) -> bool {
        let Some(snapshot) = self.undo_stack.pop() else {
            return false;
        };
        let current = PageSnapshot::capture(page);
        snapshot.restore(page);
        push_history(&mut self.redo_stack, current);
        self.selected_block = None;
        self.active_strokes.clear();
        self.pending_history = None;
        true
    }

    fn redo(&mut self, page: &mut EditorPage) -> bool {
        let Some(snapshot) = self.redo_stack.pop() else {
            return false;
        };
        let current = PageSnapshot::capture(page);
        snapshot.restore(page);
        push_history(&mut self.undo_stack, current);
        self.selected_block = None;
        self.active_strokes.clear();
        self.pending_history = None;
        true
    }
}

fn take_id(next_id: &mut u64) -> u64 {
    let id = *next_id;
    *next_id += 1;
    id
}
