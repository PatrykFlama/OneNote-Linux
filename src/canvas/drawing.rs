use super::CanvasTool;
use crate::project::{CanvasLayout, CanvasPoint, EditorBlock, EditorStroke};
use eframe::egui;

pub(super) fn pen_palette() -> [egui::Color32; 8] {
    [
        egui::Color32::from_rgb(28, 28, 30),
        egui::Color32::from_rgb(210, 45, 45),
        egui::Color32::from_rgb(230, 120, 25),
        egui::Color32::from_rgb(224, 190, 30),
        egui::Color32::from_rgb(45, 155, 75),
        egui::Color32::from_rgb(35, 125, 220),
        egui::Color32::from_rgb(105, 75, 200),
        egui::Color32::from_rgb(205, 65, 145),
    ]
}

pub(super) fn current_touch_force(ui: &egui::Ui) -> Option<f32> {
    ui.input(|input| {
        input.events.iter().rev().find_map(|event| {
            if let egui::Event::Touch {
                force: Some(force), ..
            } = event
            {
                Some(force.clamp(0.0, 1.0))
            } else {
                None
            }
        })
    })
}

pub(super) fn sampled_pen_width(
    base_width: f32,
    touch_force: Option<f32>,
    pointer_speed: f32,
    dynamics: bool,
) -> f32 {
    if !dynamics {
        return base_width;
    }
    let factor = if let Some(force) = touch_force {
        0.35 + force * 1.15
    } else {
        let speed = (pointer_speed / 1400.0).clamp(0.0, 1.0);
        1.25 - speed * 0.65
    };
    (base_width * factor).clamp(0.5, 32.0)
}

pub(super) fn smooth_stroke(stroke: &mut EditorStroke, amount: f32) {
    if stroke.points.len() < 3 || amount <= 0.0 {
        return;
    }
    let passes = if amount > 0.7 { 3 } else { 2 };
    for _ in 0..passes {
        let original = stroke.points.clone();
        for index in 1..original.len() - 1 {
            let average = CanvasPoint {
                x: (original[index - 1].x + original[index].x * 2.0 + original[index + 1].x) * 0.25,
                y: (original[index - 1].y + original[index].y * 2.0 + original[index + 1].y) * 0.25,
            };
            stroke.points[index].x = original[index].x + (average.x - original[index].x) * amount;
            stroke.points[index].y = original[index].y + (average.y - original[index].y) * amount;
        }
        if stroke.point_widths.len() == stroke.points.len() {
            let original_widths = stroke.point_widths.clone();
            for index in 1..original_widths.len() - 1 {
                let average = (original_widths[index - 1]
                    + original_widths[index] * 2.0
                    + original_widths[index + 1])
                    * 0.25;
                stroke.point_widths[index] =
                    original_widths[index] + (average - original_widths[index]) * amount;
            }
        }
    }
}

pub(super) fn shape_strokes(
    tool: CanvasTool,
    start: CanvasPoint,
    end: CanvasPoint,
    color: [u8; 4],
    width: f32,
    constrain: bool,
) -> Vec<EditorStroke> {
    let end = constrained_shape_end(tool, start, end, constrain);
    let stroke = |points| EditorStroke {
        points,
        color,
        width,
        point_widths: Vec::new(),
    };
    match tool {
        CanvasTool::Line => vec![stroke(vec![start, end])],
        CanvasTool::Arrow => {
            let dx = end.x - start.x;
            let dy = end.y - start.y;
            let length = dx.hypot(dy);
            if length <= f32::EPSILON {
                return vec![stroke(vec![start, end])];
            }
            let ux = dx / length;
            let uy = dy / length;
            let head_length = (length * 0.22).clamp(10.0, 28.0 + width);
            let wing = head_length * 0.48;
            let base = CanvasPoint {
                x: end.x - ux * head_length,
                y: end.y - uy * head_length,
            };
            let left = CanvasPoint {
                x: base.x - uy * wing,
                y: base.y + ux * wing,
            };
            let right = CanvasPoint {
                x: base.x + uy * wing,
                y: base.y - ux * wing,
            };
            vec![
                stroke(vec![start, end]),
                stroke(vec![left, end]),
                stroke(vec![right, end]),
            ]
        }
        CanvasTool::Rectangle => vec![stroke(vec![
            start,
            CanvasPoint {
                x: end.x,
                y: start.y,
            },
            end,
            CanvasPoint {
                x: start.x,
                y: end.y,
            },
            start,
        ])],
        CanvasTool::Ellipse => {
            let center = CanvasPoint {
                x: (start.x + end.x) * 0.5,
                y: (start.y + end.y) * 0.5,
            };
            let radius_x = (end.x - start.x).abs() * 0.5;
            let radius_y = (end.y - start.y).abs() * 0.5;
            let points = (0..=64)
                .map(|index| {
                    let angle = std::f32::consts::TAU * index as f32 / 64.0;
                    CanvasPoint {
                        x: center.x + angle.cos() * radius_x,
                        y: center.y + angle.sin() * radius_y,
                    }
                })
                .collect();
            vec![stroke(points)]
        }
        _ => Vec::new(),
    }
}

fn constrained_shape_end(
    tool: CanvasTool,
    start: CanvasPoint,
    end: CanvasPoint,
    constrain: bool,
) -> CanvasPoint {
    if !constrain {
        return end;
    }
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    if matches!(tool, CanvasTool::Line | CanvasTool::Arrow) {
        let length = dx.hypot(dy);
        let angle = dy.atan2(dx);
        let snapped = (angle / (std::f32::consts::FRAC_PI_4)).round() * std::f32::consts::FRAC_PI_4;
        CanvasPoint {
            x: start.x + snapped.cos() * length,
            y: start.y + snapped.sin() * length,
        }
    } else {
        let size = dx.abs().max(dy.abs());
        CanvasPoint {
            x: start.x + size.copysign(dx),
            y: start.y + size.copysign(dy),
        }
    }
}

pub(super) fn shape_is_large_enough(strokes: &[EditorStroke]) -> bool {
    let mut points = strokes.iter().flat_map(|stroke| stroke.points.iter());
    let Some(first) = points.next() else {
        return false;
    };
    let mut minimum_x = first.x;
    let mut minimum_y = first.y;
    let mut maximum_x = first.x;
    let mut maximum_y = first.y;
    for point in points {
        minimum_x = minimum_x.min(point.x);
        minimum_y = minimum_y.min(point.y);
        maximum_x = maximum_x.max(point.x);
        maximum_y = maximum_y.max(point.y);
    }
    (maximum_x - minimum_x).hypot(maximum_y - minimum_y) >= 3.0
}

pub(super) fn ink_block_from_strokes(id: u64, mut strokes: Vec<EditorStroke>) -> EditorBlock {
    let minimum_x = strokes
        .iter()
        .flat_map(|stroke| stroke.points.iter())
        .map(|point| point.x)
        .reduce(f32::min)
        .unwrap_or_default();
    let minimum_y = strokes
        .iter()
        .flat_map(|stroke| stroke.points.iter())
        .map(|point| point.y)
        .reduce(f32::min)
        .unwrap_or_default();
    let maximum_x = strokes
        .iter()
        .flat_map(|stroke| stroke.points.iter())
        .map(|point| point.x)
        .reduce(f32::max)
        .unwrap_or(minimum_x);
    let maximum_y = strokes
        .iter()
        .flat_map(|stroke| stroke.points.iter())
        .map(|point| point.y)
        .reduce(f32::max)
        .unwrap_or(minimum_y);
    let padding = strokes
        .iter()
        .flat_map(|stroke| std::iter::once(stroke.width).chain(stroke.point_widths.iter().copied()))
        .reduce(f32::max)
        .unwrap_or(1.0)
        + 4.0;
    for stroke in &mut strokes {
        for point in &mut stroke.points {
            point.x = point.x - minimum_x + padding;
            point.y = point.y - minimum_y + padding;
        }
    }
    let stroke_count = strokes.len();
    EditorBlock::Ink {
        id,
        stroke_count,
        strokes,
        layout: CanvasLayout::new(
            minimum_x - padding,
            minimum_y - padding,
            maximum_x - minimum_x + padding * 2.0,
            maximum_y - minimum_y + padding * 2.0,
        ),
    }
}

pub(super) fn erase_at(blocks: &mut Vec<EditorBlock>, position: egui::Pos2, radius: f32) -> bool {
    let mut changed = false;
    for block in blocks.iter_mut() {
        if let EditorBlock::Ink {
            stroke_count,
            strokes,
            layout,
            ..
        } = block
        {
            let mut erased_strokes = Vec::with_capacity(strokes.len());
            let mut block_changed = false;
            for stroke in strokes.drain(..) {
                if stroke_hits_eraser(&stroke, *layout, position, radius) {
                    erased_strokes
                        .extend(erase_stroke_segments(&stroke, *layout, position, radius));
                    block_changed = true;
                } else {
                    erased_strokes.push(stroke);
                }
            }
            *strokes = erased_strokes;
            if block_changed {
                *stroke_count = strokes.len();
                changed = true;
            }
        }
    }
    blocks.retain(|block| !matches!(block, EditorBlock::Ink { strokes, .. } if strokes.is_empty()));
    changed
}

pub(super) fn ink_at(blocks: &[EditorBlock], position: egui::Pos2, radius: f32) -> bool {
    blocks.iter().any(|block| {
        let EditorBlock::Ink {
            strokes, layout, ..
        } = block
        else {
            return false;
        };
        strokes
            .iter()
            .any(|stroke| stroke_hits_eraser(stroke, *layout, position, radius))
    })
}

fn erase_stroke_segments(
    stroke: &EditorStroke,
    layout: CanvasLayout,
    position: egui::Pos2,
    radius: f32,
) -> Vec<EditorStroke> {
    let effective_radius = radius + stroke_max_width(stroke) * 0.5;
    let radius_squared = effective_radius * effective_radius;
    if !stroke_hits_eraser(stroke, layout, position, radius) {
        return vec![stroke.clone()];
    }
    if stroke.points.len() < 2 {
        let hit = stroke.points.first().is_some_and(|point| {
            let x = layout.x + point.x - position.x;
            let y = layout.y + point.y - position.y;
            x * x + y * y <= radius_squared
        });
        return if hit {
            Vec::new()
        } else {
            vec![stroke.clone()]
        };
    }
    let has_dynamic_width = stroke.point_widths.len() == stroke.points.len();
    let mut dense_points = Vec::new();
    let mut dense_widths = Vec::new();
    let maximum_step = (radius * 0.4).max(2.0);

    dense_points.push(stroke.points[0]);
    if has_dynamic_width {
        dense_widths.push(stroke.point_widths[0]);
    }
    for index in 1..stroke.points.len() {
        let start = stroke.points[index - 1];
        let end = stroke.points[index];
        let distance = (end.x - start.x).hypot(end.y - start.y);
        let steps = (distance / maximum_step).ceil().max(1.0) as usize;
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            dense_points.push(CanvasPoint {
                x: start.x + (end.x - start.x) * t,
                y: start.y + (end.y - start.y) * t,
            });
            if has_dynamic_width {
                dense_widths.push(
                    stroke.point_widths[index - 1]
                        + (stroke.point_widths[index] - stroke.point_widths[index - 1]) * t,
                );
            }
        }
    }

    let erased = dense_points.iter().map(|point| {
        let x = layout.x + point.x - position.x;
        let y = layout.y + point.y - position.y;
        x * x + y * y <= radius_squared
    });
    let mut pieces = Vec::new();
    let mut points = Vec::new();
    let mut widths = Vec::new();
    for (index, is_erased) in erased.enumerate() {
        if is_erased {
            finish_erased_piece(stroke, &mut points, &mut widths, &mut pieces);
        } else {
            points.push(dense_points[index]);
            if has_dynamic_width {
                widths.push(dense_widths[index]);
            }
        }
    }
    finish_erased_piece(stroke, &mut points, &mut widths, &mut pieces);
    pieces
}

fn stroke_hits_eraser(
    stroke: &EditorStroke,
    layout: CanvasLayout,
    position: egui::Pos2,
    radius: f32,
) -> bool {
    let effective_radius = radius + stroke_max_width(stroke) * 0.5;
    if let [point] = stroke.points.as_slice() {
        let absolute = egui::pos2(layout.x + point.x, layout.y + point.y);
        return position.distance(absolute) <= effective_radius;
    }
    stroke.points.windows(2).any(|points| {
        let start = egui::pos2(layout.x + points[0].x, layout.y + points[0].y);
        let end = egui::pos2(layout.x + points[1].x, layout.y + points[1].y);
        distance_to_segment(position, start, end) <= effective_radius
    })
}

fn stroke_max_width(stroke: &EditorStroke) -> f32 {
    stroke
        .point_widths
        .iter()
        .copied()
        .fold(stroke.width, f32::max)
}

fn distance_to_segment(point: egui::Pos2, start: egui::Pos2, end: egui::Pos2) -> f32 {
    let segment = end - start;
    let length_squared = segment.length_sq();
    if length_squared <= f32::EPSILON {
        return point.distance(start);
    }
    let t = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
    point.distance(start + segment * t)
}

fn finish_erased_piece(
    source: &EditorStroke,
    points: &mut Vec<CanvasPoint>,
    widths: &mut Vec<f32>,
    output: &mut Vec<EditorStroke>,
) {
    if points.len() >= 2 {
        output.push(EditorStroke {
            points: std::mem::take(points),
            color: source.color,
            width: source.width,
            point_widths: std::mem::take(widths),
        });
    } else {
        points.clear();
        widths.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_stroke(points: Vec<CanvasPoint>) -> EditorStroke {
        EditorStroke {
            points,
            color: [0, 0, 0, 255],
            width: 2.0,
            point_widths: Vec::new(),
        }
    }

    #[test]
    fn drawn_stroke_is_normalized_into_its_block() {
        let block = ink_block_from_strokes(
            1,
            vec![test_stroke(vec![
                CanvasPoint { x: 100.0, y: 80.0 },
                CanvasPoint { x: 140.0, y: 120.0 },
            ])],
        );
        let EditorBlock::Ink {
            layout, strokes, ..
        } = block
        else {
            panic!("expected ink block");
        };
        assert!(layout.x < 100.0);
        assert!(strokes[0].points[0].x > 0.0);
    }

    #[test]
    fn arrow_is_stored_as_editable_ink_strokes() {
        let strokes = shape_strokes(
            CanvasTool::Arrow,
            CanvasPoint { x: 10.0, y: 10.0 },
            CanvasPoint { x: 110.0, y: 40.0 },
            [20, 30, 40, 255],
            3.0,
            false,
        );
        assert_eq!(strokes.len(), 3);
        assert!(strokes.iter().all(|stroke| stroke.points.len() == 2));
        assert!(shape_is_large_enough(&strokes));
    }

    #[test]
    fn shift_constrains_rectangles_to_squares() {
        let end = constrained_shape_end(
            CanvasTool::Rectangle,
            CanvasPoint { x: 10.0, y: 20.0 },
            CanvasPoint { x: 50.0, y: 90.0 },
            true,
        );
        assert_eq!(end, CanvasPoint { x: 80.0, y: 90.0 });
    }

    #[test]
    fn eraser_splits_a_stroke_instead_of_deleting_all_of_it() {
        let stroke = test_stroke(vec![
            CanvasPoint { x: 0.0, y: 20.0 },
            CanvasPoint { x: 100.0, y: 20.0 },
        ]);
        let pieces = erase_stroke_segments(
            &stroke,
            CanvasLayout::new(0.0, 0.0, 100.0, 40.0),
            egui::pos2(50.0, 20.0),
            8.0,
        );
        assert_eq!(pieces.len(), 2);
        assert!(pieces[0].points.last().unwrap().x < 50.0);
        assert!(pieces[1].points.first().unwrap().x > 50.0);
    }

    #[test]
    fn eraser_does_not_resample_unaffected_strokes() {
        let stroke = test_stroke(vec![
            CanvasPoint { x: 0.0, y: 20.0 },
            CanvasPoint { x: 100.0, y: 20.0 },
        ]);
        let pieces = erase_stroke_segments(
            &stroke,
            CanvasLayout::new(0.0, 0.0, 100.0, 40.0),
            egui::pos2(50.0, 80.0),
            8.0,
        );
        assert_eq!(pieces, vec![stroke]);
    }

    #[test]
    fn pressure_and_speed_change_dynamic_pen_width() {
        assert!(sampled_pen_width(4.0, Some(1.0), 0.0, true) > 4.0);
        assert!(
            sampled_pen_width(4.0, None, 0.0, true) > sampled_pen_width(4.0, None, 1400.0, true)
        );
        assert_eq!(sampled_pen_width(4.0, Some(0.1), 500.0, false), 4.0);
    }
}
