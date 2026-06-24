use super::{
    Asset, CanvasLayout, CanvasPoint, EditorBlock, EditorPage, EditorSection, EditorStroke,
    FORMAT_VERSION, HALF_INCH_TO_PIXELS, INK_UNIT_TO_PIXELS, Project, default_canvas_height,
    default_canvas_width, estimated_block_height,
};
use libonenote::{
    Content, Document, Ink as OneNoteInk, Layout as OneNoteLayout, NotebookEntry, OutlineElement,
    OutlineItem, PageBlock, Paragraph,
};
use sanitize_filename::{Options as FilenameOptions, sanitize_with_options};

impl Project {
    pub fn import(document: &Document) -> Self {
        let mut builder = ProjectBuilder {
            project: Self {
                format_version: FORMAT_VERSION,
                name: document.notebook().name.clone(),
                source: document
                    .source()
                    .path
                    .as_ref()
                    .map(|path| path.display().to_string()),
                sections: Vec::new(),
                assets: Vec::new(),
                next_id: 1,
                graph_sync: Default::default(),
            },
        };

        for entry in &document.notebook().entries {
            builder.import_entry(entry, &[]);
        }
        builder.project
    }
}

struct ProjectBuilder {
    project: Project,
}

impl ProjectBuilder {
    fn id(&mut self) -> u64 {
        self.project.allocate_id()
    }

    fn import_entry(&mut self, entry: &NotebookEntry, group_path: &[String]) {
        match entry {
            NotebookEntry::Section(section) => {
                let mut path = group_path.to_vec();
                path.push(section.name.clone());
                let name = path.join(" / ");
                let pages = section
                    .pages
                    .iter()
                    .map(|page| {
                        let mut blocks = Vec::new();
                        for block in &page.blocks {
                            self.import_page_block(block, &mut blocks);
                        }
                        let (canvas_width, canvas_height) = canvas_size(&blocks, page.height);
                        EditorPage {
                            id: self.id(),
                            title: page.title.clone(),
                            level: page.level,
                            author: page.author.clone(),
                            created_at: page.created_at.clone(),
                            updated_at: page.updated_at.clone(),
                            canvas_width,
                            canvas_height,
                            blocks,
                        }
                    })
                    .collect();
                let id = self.id();
                self.project
                    .sections
                    .push(EditorSection { id, name, pages });
            }
            NotebookEntry::SectionGroup(group) => {
                let mut path = group_path.to_vec();
                path.push(group.name.clone());
                for child in &group.entries {
                    self.import_entry(child, &path);
                }
            }
        }
    }

    fn import_page_block(&mut self, block: &PageBlock, output: &mut Vec<EditorBlock>) {
        match block {
            PageBlock::Outline(outline) => {
                let mut blocks = Vec::new();
                for item in &outline.items {
                    self.import_outline_item(item, &mut blocks);
                }
                merge_adjacent_text_blocks(&mut blocks);
                let fallback = CanvasLayout::new(96.0, 120.0, 560.0, 180.0);
                let layout = one_note_layout(&outline.layout, fallback);
                position_flow(&mut blocks, layout);
                output.extend(blocks);
            }
            PageBlock::Image(image) => {
                let asset_id = self.add_asset(
                    image.filename.as_deref().unwrap_or("image.bin"),
                    image.blob.bytes(),
                    image.blob.size,
                );
                output.push(EditorBlock::Image {
                    id: self.id(),
                    asset_id,
                    layout: one_note_layout(
                        &image.layout,
                        CanvasLayout::new(96.0, 120.0, 480.0, 320.0),
                    ),
                });
            }
            PageBlock::Attachment(file) => {
                let asset_id = self.add_asset(&file.filename, file.blob.bytes(), file.blob.size);
                output.push(EditorBlock::Attachment {
                    id: self.id(),
                    asset_id,
                    layout: one_note_layout(
                        &file.layout,
                        CanvasLayout::new(96.0, 120.0, 280.0, 72.0),
                    ),
                });
            }
            PageBlock::Ink(ink) => output.push(self.import_ink(ink, true)),
            PageBlock::Unknown => output.push(EditorBlock::Unsupported {
                id: self.id(),
                description: "unknown page content".to_owned(),
                layout: CanvasLayout::default(),
            }),
        }
    }

    fn import_outline_item(&mut self, item: &OutlineItem, output: &mut Vec<EditorBlock>) {
        match item {
            OutlineItem::Element(element) => self.import_element(element, output),
            OutlineItem::Group(group) => {
                for child in &group.items {
                    self.import_outline_item(child, output);
                }
            }
        }
    }

    fn import_element(&mut self, element: &OutlineElement, output: &mut Vec<EditorBlock>) {
        for content in &element.content {
            match content {
                Content::Paragraph(paragraph) => {
                    let text = visible_text(paragraph);
                    if !text.trim().is_empty() {
                        output.push(EditorBlock::Text {
                            id: self.id(),
                            text,
                            indent: element.level.saturating_sub(1),
                            layout: CanvasLayout::default(),
                        });
                    }
                }
                Content::Table(table) => {
                    let rows = table
                        .content
                        .iter()
                        .map(|row| {
                            row.cells
                                .iter()
                                .map(|cell| {
                                    let mut text = String::new();
                                    for element in &cell.content {
                                        append_element_text(element, &mut text);
                                    }
                                    text
                                })
                                .collect()
                        })
                        .collect();
                    output.push(EditorBlock::Table {
                        id: self.id(),
                        rows,
                        layout: CanvasLayout::default(),
                    });
                }
                Content::Image(image) => {
                    let asset_id = self.add_asset(
                        image.filename.as_deref().unwrap_or("image.bin"),
                        image.blob.bytes(),
                        image.blob.size,
                    );
                    output.push(EditorBlock::Image {
                        id: self.id(),
                        asset_id,
                        layout: CanvasLayout::default(),
                    });
                }
                Content::Attachment(file) => {
                    let asset_id =
                        self.add_asset(&file.filename, file.blob.bytes(), file.blob.size);
                    output.push(EditorBlock::Attachment {
                        id: self.id(),
                        asset_id,
                        layout: CanvasLayout::default(),
                    });
                }
                Content::Ink(ink) => output.push(self.import_ink(ink, false)),
                Content::Unknown => output.push(EditorBlock::Unsupported {
                    id: self.id(),
                    description: "unknown outline content".to_owned(),
                    layout: CanvasLayout::default(),
                }),
            }
        }
        for child in &element.children {
            self.import_outline_item(child, output);
        }
    }

    fn import_ink(&mut self, ink: &OneNoteInk, positioned_on_page: bool) -> EditorBlock {
        let mut strokes = Vec::new();
        collect_ink_strokes(ink, positioned_on_page, &mut strokes);
        let origin = normalize_strokes(&mut strokes).unwrap_or(CanvasPoint { x: 96.0, y: 120.0 });
        let bounds = stroke_bounds(&strokes);
        let native_size = ink_layout(
            &ink.layout,
            CanvasLayout::new(origin.x, origin.y, bounds.0, bounds.1),
        );
        let layout = CanvasLayout::new(
            if positioned_on_page { origin.x } else { 0.0 },
            if positioned_on_page { origin.y } else { 0.0 },
            native_size.width.max(bounds.0),
            native_size.height.max(bounds.1),
        );
        EditorBlock::Ink {
            id: self.id(),
            stroke_count: ink.stroke_count,
            strokes,
            layout,
        }
    }

    fn add_asset(&mut self, filename: &str, bytes: Option<&[u8]>, size: u64) -> u64 {
        let id = self.id();
        let safe_name = safe_filename(filename);
        let stored_name = format!("{id:08}-{safe_name}");
        self.project.assets.push(Asset {
            id,
            filename: filename.to_owned(),
            media_type: mime_guess::from_path(filename)
                .first_or_octet_stream()
                .to_string(),
            stored_name,
            size,
            bytes: bytes.unwrap_or_default().to_vec(),
        });
        id
    }
}

fn one_note_layout(layout: &OneNoteLayout, fallback: CanvasLayout) -> CanvasLayout {
    CanvasLayout::new(
        layout
            .x
            .map_or(fallback.x, |value| value * HALF_INCH_TO_PIXELS),
        layout
            .y
            .map_or(fallback.y, |value| value * HALF_INCH_TO_PIXELS),
        layout
            .width
            .map_or(fallback.width, |value| value * HALF_INCH_TO_PIXELS),
        layout
            .height
            .map_or(fallback.height, |value| value * HALF_INCH_TO_PIXELS),
    )
}

fn ink_layout(layout: &OneNoteLayout, fallback: CanvasLayout) -> CanvasLayout {
    CanvasLayout::new(
        layout
            .x
            .map_or(fallback.x, |value| value * HALF_INCH_TO_PIXELS),
        layout
            .y
            .map_or(fallback.y, |value| value * HALF_INCH_TO_PIXELS),
        layout
            .width
            .map_or(fallback.width, |value| value * INK_UNIT_TO_PIXELS),
        layout
            .height
            .map_or(fallback.height, |value| value * INK_UNIT_TO_PIXELS),
    )
}

pub(super) fn collect_ink_strokes(
    ink: &OneNoteInk,
    include_layout_offset: bool,
    output: &mut Vec<EditorStroke>,
) {
    let offset_x = if include_layout_offset {
        ink.layout.x.unwrap_or_default() * HALF_INCH_TO_PIXELS
    } else {
        0.0
    };
    let offset_y = if include_layout_offset {
        ink.layout.y.unwrap_or_default() * HALF_INCH_TO_PIXELS
    } else {
        0.0
    };

    for stroke in &ink.strokes {
        if stroke.points.is_empty() {
            continue;
        }
        let packed = stroke.color.unwrap_or_default();
        let alpha = 255_u8.saturating_sub(stroke.transparency.unwrap_or_default());
        output.push(EditorStroke {
            points: decode_ink_points(&stroke.points, offset_x, offset_y),
            color: [
                (packed & 0xff) as u8,
                ((packed >> 8) & 0xff) as u8,
                ((packed >> 16) & 0xff) as u8,
                alpha,
            ],
            width: (stroke.width * INK_UNIT_TO_PIXELS).clamp(1.2, 32.0),
            point_widths: Vec::new(),
        });
    }

    for child in &ink.groups {
        collect_ink_strokes(child, include_layout_offset, output);
    }
}

fn decode_ink_points(
    points: &[libonenote::Point],
    offset_x: f32,
    offset_y: f32,
) -> Vec<CanvasPoint> {
    let Some((start, deltas)) = points.split_first() else {
        return Vec::new();
    };
    let mut x = offset_x + start.x * INK_UNIT_TO_PIXELS;
    let mut y = offset_y + start.y * INK_UNIT_TO_PIXELS;
    let mut decoded = Vec::with_capacity(points.len());
    decoded.push(CanvasPoint { x, y });
    for delta in deltas {
        x += delta.x * INK_UNIT_TO_PIXELS;
        y += delta.y * INK_UNIT_TO_PIXELS;
        decoded.push(CanvasPoint { x, y });
    }
    decoded
}

pub(super) fn normalize_strokes(strokes: &mut [EditorStroke]) -> Option<CanvasPoint> {
    let minimum_x = strokes
        .iter()
        .flat_map(|stroke| &stroke.points)
        .map(|point| point.x)
        .reduce(f32::min);
    let minimum_y = strokes
        .iter()
        .flat_map(|stroke| &stroke.points)
        .map(|point| point.y)
        .reduce(f32::min);
    let (Some(minimum_x), Some(minimum_y)) = (minimum_x, minimum_y) else {
        return None;
    };
    for point in strokes.iter_mut().flat_map(|stroke| &mut stroke.points) {
        point.x -= minimum_x;
        point.y -= minimum_y;
    }
    Some(CanvasPoint {
        x: minimum_x,
        y: minimum_y,
    })
}

fn stroke_bounds(strokes: &[EditorStroke]) -> (f32, f32) {
    let width = strokes
        .iter()
        .flat_map(|stroke| &stroke.points)
        .map(|point| point.x)
        .reduce(f32::max)
        .unwrap_or(320.0);
    let height = strokes
        .iter()
        .flat_map(|stroke| &stroke.points)
        .map(|point| point.y)
        .reduce(f32::max)
        .unwrap_or(180.0);
    (width + 8.0, height + 8.0)
}

fn position_flow(blocks: &mut [EditorBlock], container: CanvasLayout) {
    let mut y = container.y;
    for block in blocks {
        let indent = match block {
            EditorBlock::Text { indent, .. } => *indent as f32 * 22.0,
            _ => 0.0,
        };
        let height = estimated_block_height(block);
        *block.layout_mut() = CanvasLayout::new(
            container.x + indent,
            y,
            (container.width - indent).max(160.0),
            height,
        );
        y += height + 12.0;
    }
}

pub(super) fn merge_adjacent_text_blocks(blocks: &mut Vec<EditorBlock>) {
    let mut merged = Vec::with_capacity(blocks.len());
    for block in blocks.drain(..) {
        match (merged.last_mut(), block) {
            (
                Some(EditorBlock::Text { text: previous, .. }),
                EditorBlock::Text { text, indent, .. },
            ) => {
                if !previous.is_empty() {
                    previous.push('\n');
                }
                previous.push_str(&"  ".repeat(indent as usize));
                previous.push_str(&text);
            }
            (_, block) => merged.push(block),
        }
    }
    *blocks = merged;
}

fn canvas_size(blocks: &[EditorBlock], page_height: Option<f32>) -> (f32, f32) {
    let width = blocks
        .iter()
        .map(|block| block.layout().x + block.layout().width)
        .reduce(f32::max)
        .unwrap_or(default_canvas_width());
    let content_height = blocks
        .iter()
        .map(|block| block.layout().y + block.layout().height)
        .reduce(f32::max)
        .unwrap_or(default_canvas_height());
    let native_height = page_height.unwrap_or_default() * HALF_INCH_TO_PIXELS;
    (
        default_canvas_width().max(width + 240.0),
        default_canvas_height().max(content_height.max(native_height) + 240.0),
    )
}

fn visible_text(paragraph: &Paragraph) -> String {
    if paragraph.runs.is_empty() {
        return paragraph.text.clone();
    }
    paragraph
        .runs
        .iter()
        .filter(|run| !run.style.hidden)
        .map(|run| run.text.as_str())
        .collect()
}

fn append_element_text(element: &OutlineElement, output: &mut String) {
    for content in &element.content {
        if let Content::Paragraph(paragraph) = content {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&visible_text(paragraph));
        }
    }
    for child in &element.children {
        match child {
            OutlineItem::Element(element) => append_element_text(element, output),
            OutlineItem::Group(group) => {
                for child in &group.items {
                    if let OutlineItem::Element(element) = child {
                        append_element_text(element, output);
                    }
                }
            }
        }
    }
}

pub(super) fn safe_filename(filename: &str) -> String {
    let filename = filename
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("asset.bin");
    let name = sanitize_with_options(
        filename,
        FilenameOptions {
            windows: false,
            ..FilenameOptions::default()
        },
    );
    if name.is_empty() {
        "asset.bin".to_owned()
    } else {
        name
    }
}
