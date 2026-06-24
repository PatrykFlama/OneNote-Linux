use super::CanvasTool;
use crate::project::{Asset, CanvasLayout, EditorBlock, EditorPage, EditorStroke};
use eframe::egui;
use rfd::FileDialog;
use std::fs;

pub(super) fn render_positioned_block(
    ui: &mut egui::Ui,
    block: &mut EditorBlock,
    assets: &[Asset],
    tool: CanvasTool,
    selected_block: &mut Option<u64>,
    error: &mut Option<String>,
) -> bool {
    let mut changed = false;
    let id = block.id();
    let layout = block.layout();
    let rect = layout_rect(layout);
    let header = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.min.y + 24.0));
    let content = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + 6.0, header.max.y + 4.0),
        egui::pos2(rect.max.x - 6.0, rect.max.y - 6.0),
    );

    ui.painter().rect_filled(rect, 3.0, egui::Color32::WHITE);
    ui.painter().rect_stroke(
        rect,
        3.0,
        egui::Stroke::new(1.0, egui::Color32::from_gray(205)),
        egui::StrokeKind::Inside,
    );

    if tool == CanvasTool::Select {
        let move_response = ui
            .interact(
                header,
                ui.id().with(("move", id)),
                egui::Sense::click_and_drag(),
            )
            .on_hover_cursor(egui::CursorIcon::Grab);
        if move_response.clicked() {
            *selected_block = Some(id);
        }
        if move_response.dragged() {
            let delta = move_response.drag_delta();
            let layout = block.layout_mut();
            layout.x += delta.x;
            layout.y += delta.y;
            changed = true;
        }

        let resize_rect = egui::Rect::from_min_size(
            egui::pos2(rect.max.x - 14.0, rect.max.y - 14.0),
            egui::vec2(14.0, 14.0),
        );
        let resize_response = ui
            .interact(
                resize_rect,
                ui.id().with(("resize", id)),
                egui::Sense::drag(),
            )
            .on_hover_cursor(egui::CursorIcon::ResizeNwSe);
        ui.painter()
            .rect_filled(resize_rect, 2.0, egui::Color32::from_gray(185));
        if resize_response.dragged() {
            let delta = resize_response.drag_delta();
            let layout = block.layout_mut();
            layout.width = (layout.width + delta.x).max(100.0);
            layout.height = (layout.height + delta.y).max(56.0);
            changed = true;
        }
    }

    ui.painter().text(
        header.left_center() + egui::vec2(7.0, 0.0),
        egui::Align2::LEFT_CENTER,
        block_label(block),
        egui::FontId::proportional(12.0),
        egui::Color32::from_gray(110),
    );

    match block {
        EditorBlock::Text { text, .. } => {
            changed |= ui
                .put(
                    content,
                    egui::TextEdit::multiline(text)
                        .desired_width(f32::INFINITY)
                        .text_color(egui::Color32::from_rgb(28, 28, 30))
                        .background_color(egui::Color32::WHITE),
                )
                .changed();
        }
        EditorBlock::Table { id, rows, .. } => {
            let columns = rows.iter().map(Vec::len).max().unwrap_or(2).max(1);
            for row in rows.iter_mut() {
                row.resize(columns, String::new());
            }
            let response = ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(content)
                    .id_salt(("table", *id)),
                |ui| {
                    egui::Grid::new(("canvas_table", *id))
                        .striped(true)
                        .show(ui, |ui| {
                            for row in rows.iter_mut() {
                                for cell in row.iter_mut() {
                                    changed |= ui
                                        .add(
                                            egui::TextEdit::singleline(cell)
                                                .desired_width(140.0)
                                                .text_color(egui::Color32::from_rgb(28, 28, 30))
                                                .background_color(egui::Color32::WHITE),
                                        )
                                        .changed();
                                }
                                ui.end_row();
                            }
                        });
                    ui.horizontal(|ui| {
                        if ui.small_button("+ Row").clicked() {
                            rows.push(vec![String::new(); columns]);
                            changed = true;
                        }
                        if ui.small_button("+ Column").clicked() {
                            for row in rows.iter_mut() {
                                row.push(String::new());
                            }
                            changed = true;
                        }
                    });
                },
            );
            changed |= response.response.changed();
        }
        EditorBlock::Image { asset_id, .. } => {
            if let Some(asset) = assets.iter().find(|asset| asset.id == *asset_id) {
                if asset.bytes.is_empty() {
                    ui.painter().text(
                        content.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("{}\nimage data not loaded", asset.filename),
                        egui::FontId::proportional(14.0),
                        egui::Color32::DARK_GRAY,
                    );
                } else {
                    ui.put(
                        content,
                        egui::Image::from_bytes(
                            format!("bytes://asset/{}", asset.id),
                            asset.bytes.clone(),
                        )
                        .fit_to_exact_size(content.size()),
                    );
                }
            }
        }
        EditorBlock::Attachment { asset_id, .. } => {
            if let Some(asset) = assets.iter().find(|asset| asset.id == *asset_id) {
                ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
                    ui.label(format!("{} ({} bytes)", asset.filename, asset.size));
                    if ui
                        .add_enabled(
                            !asset.bytes.is_empty(),
                            egui::Button::new("Save attachment…"),
                        )
                        .clicked()
                        && let Some(path) =
                            FileDialog::new().set_file_name(&asset.filename).save_file()
                        && let Err(save_error) = fs::write(path, &asset.bytes)
                    {
                        *error = Some(save_error.to_string());
                    }
                });
            }
        }
        EditorBlock::Unsupported { description, .. } => {
            ui.painter().text(
                content.left_top(),
                egui::Align2::LEFT_TOP,
                format!("Unsupported OneNote object: {description}"),
                egui::FontId::proportional(13.0),
                egui::Color32::DARK_GRAY,
            );
        }
        EditorBlock::Ink { .. } => {}
    }

    changed
}

pub(super) fn paint_page_background(painter: &egui::Painter, rect: egui::Rect) {
    painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(250, 250, 248));
    let grid = egui::Color32::from_gray(232);
    let step = 48.0;
    let mut x = step;
    while x < rect.max.x {
        painter.line_segment(
            [egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)],
            egui::Stroke::new(0.5, grid),
        );
        x += step;
    }
    let mut y = step;
    while y < rect.max.y {
        painter.line_segment(
            [egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)],
            egui::Stroke::new(0.5, grid),
        );
        y += step;
    }
}

pub(super) fn paint_ink(painter: &egui::Painter, layout: CanvasLayout, strokes: &[EditorStroke]) {
    for stroke in strokes {
        paint_stroke(painter, egui::pos2(layout.x, layout.y), stroke);
    }
}

pub(super) fn paint_stroke(painter: &egui::Painter, origin: egui::Pos2, stroke: &EditorStroke) {
    if stroke.points.len() < 2 {
        return;
    }
    let color = egui::Color32::from_rgba_unmultiplied(
        stroke.color[0],
        stroke.color[1],
        stroke.color[2],
        stroke.color[3],
    );
    if stroke.point_widths.len() == stroke.points.len() {
        let first = stroke.points[0];
        painter.circle_filled(
            egui::pos2(origin.x + first.x, origin.y + first.y),
            stroke.point_widths[0] * 0.5,
            color,
        );
        for (index, points) in stroke.points.windows(2).enumerate() {
            let start = egui::pos2(origin.x + points[0].x, origin.y + points[0].y);
            let end = egui::pos2(origin.x + points[1].x, origin.y + points[1].y);
            let width = (stroke.point_widths[index] + stroke.point_widths[index + 1]) * 0.5;
            painter.line_segment([start, end], egui::Stroke::new(width, color));
            if color.a() == 255 {
                painter.circle_filled(end, width * 0.5, color);
            }
        }
    } else {
        let points = stroke
            .points
            .iter()
            .map(|point| egui::pos2(origin.x + point.x, origin.y + point.y))
            .collect();
        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(stroke.width, color),
        ));
    }
}

pub(super) fn expand_canvas(page: &mut EditorPage) {
    for block in &page.blocks {
        let layout = block.layout();
        page.canvas_width = page.canvas_width.max(layout.x + layout.width + 240.0);
        page.canvas_height = page.canvas_height.max(layout.y + layout.height + 240.0);
    }
}

pub(super) fn layout_rect(layout: CanvasLayout) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(layout.x, layout.y),
        egui::vec2(layout.width, layout.height),
    )
}

fn block_label(block: &EditorBlock) -> &'static str {
    match block {
        EditorBlock::Text { .. } => "Text",
        EditorBlock::Table { .. } => "Table",
        EditorBlock::Image { .. } => "Image",
        EditorBlock::Attachment { .. } => "Attachment",
        EditorBlock::Ink { .. } => "Ink",
        EditorBlock::Unsupported { .. } => "Unsupported",
    }
}
