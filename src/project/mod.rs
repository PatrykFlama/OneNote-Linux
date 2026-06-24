mod graph;
mod import;
mod native;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub(crate) use graph::{GRAPH_SYNC_VERSION, GraphSyncPage};

#[cfg(test)]
use import::{collect_ink_strokes, merge_adjacent_text_blocks, normalize_strokes, safe_filename};
#[cfg(test)]
use libonenote::{Ink as OneNoteInk, Layout as OneNoteLayout};

const FORMAT_VERSION: u32 = 2;
pub(super) const HALF_INCH_TO_PIXELS: f32 = 48.0;
pub(super) const INK_UNIT_TO_PIXELS: f32 = HALF_INCH_TO_PIXELS / 1270.0;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub format_version: u32,
    pub name: String,
    pub source: Option<String>,
    pub sections: Vec<EditorSection>,
    pub assets: Vec<Asset>,
    pub next_id: u64,
    #[serde(default)]
    pub graph_sync: GraphSyncMetadata,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GraphSyncMetadata {
    pub notebook_id: Option<String>,
    pub notebook_name: Option<String>,
    pub section_id: Option<String>,
    pub section_name: Option<String>,
    pub pages: BTreeMap<u64, GraphPageLink>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphPageLink {
    pub graph_page_id: String,
    pub uploaded_at: String,
    #[serde(default)]
    pub sync_version: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EditorSection {
    pub id: u64,
    pub name: String,
    pub pages: Vec<EditorPage>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EditorPage {
    pub id: u64,
    pub title: String,
    pub level: i32,
    pub author: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default = "default_canvas_width")]
    pub canvas_width: f32,
    #[serde(default = "default_canvas_height")]
    pub canvas_height: f32,
    pub blocks: Vec<EditorBlock>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanvasLayout {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl CanvasLayout {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width: width.max(32.0),
            height: height.max(24.0),
        }
    }
}

impl Default for CanvasLayout {
    fn default() -> Self {
        Self::new(96.0, 120.0, 440.0, 140.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanvasPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditorStroke {
    pub points: Vec<CanvasPoint>,
    pub color: [u8; 4],
    pub width: f32,
    /// Optional width for each point. Empty means a constant-width stroke.
    #[serde(default)]
    pub point_widths: Vec<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EditorBlock {
    Text {
        id: u64,
        text: String,
        indent: u8,
        #[serde(default)]
        layout: CanvasLayout,
    },
    Table {
        id: u64,
        rows: Vec<Vec<String>>,
        #[serde(default)]
        layout: CanvasLayout,
    },
    Image {
        id: u64,
        asset_id: u64,
        #[serde(default)]
        layout: CanvasLayout,
    },
    Attachment {
        id: u64,
        asset_id: u64,
        #[serde(default)]
        layout: CanvasLayout,
    },
    Ink {
        id: u64,
        stroke_count: usize,
        #[serde(default)]
        strokes: Vec<EditorStroke>,
        #[serde(default)]
        layout: CanvasLayout,
    },
    Unsupported {
        id: u64,
        description: String,
        #[serde(default)]
        layout: CanvasLayout,
    },
}

impl EditorBlock {
    pub fn id(&self) -> u64 {
        match self {
            Self::Text { id, .. }
            | Self::Table { id, .. }
            | Self::Image { id, .. }
            | Self::Attachment { id, .. }
            | Self::Ink { id, .. }
            | Self::Unsupported { id, .. } => *id,
        }
    }

    pub fn layout(&self) -> CanvasLayout {
        match self {
            Self::Text { layout, .. }
            | Self::Table { layout, .. }
            | Self::Image { layout, .. }
            | Self::Attachment { layout, .. }
            | Self::Ink { layout, .. }
            | Self::Unsupported { layout, .. } => *layout,
        }
    }

    pub fn layout_mut(&mut self) -> &mut CanvasLayout {
        match self {
            Self::Text { layout, .. }
            | Self::Table { layout, .. }
            | Self::Image { layout, .. }
            | Self::Attachment { layout, .. }
            | Self::Ink { layout, .. }
            | Self::Unsupported { layout, .. } => layout,
        }
    }
}

pub(super) fn default_canvas_width() -> f32 {
    1600.0
}

pub(super) fn default_canvas_height() -> f32 {
    1200.0
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Asset {
    pub id: u64,
    pub filename: String,
    pub media_type: String,
    pub stored_name: String,
    pub size: u64,
    #[serde(skip, default)]
    pub bytes: Vec<u8>,
}

impl Project {
    pub fn empty() -> Self {
        let mut project = Self {
            format_version: FORMAT_VERSION,
            name: "New notebook".to_owned(),
            source: None,
            sections: Vec::new(),
            assets: Vec::new(),
            next_id: 1,
            graph_sync: GraphSyncMetadata::default(),
        };
        let section_id = project.allocate_id();
        let page_id = project.allocate_id();
        let block_id = project.allocate_id();
        project.sections.push(EditorSection {
            id: section_id,
            name: "Section".to_owned(),
            pages: vec![EditorPage {
                id: page_id,
                title: "New page".to_owned(),
                level: 0,
                author: None,
                created_at: String::new(),
                updated_at: String::new(),
                canvas_width: default_canvas_width(),
                canvas_height: default_canvas_height(),
                blocks: vec![EditorBlock::Text {
                    id: block_id,
                    text: String::new(),
                    indent: 0,
                    layout: CanvasLayout::default(),
                }],
            }],
        });
        project
    }

    pub fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn asset(&self, id: u64) -> Option<&Asset> {
        self.assets.iter().find(|asset| asset.id == id)
    }

    fn migrate_legacy_canvas(&mut self) {
        for section in &mut self.sections {
            for page in &mut section.pages {
                let mut y = 120.0;
                for block in &mut page.blocks {
                    let height = estimated_block_height(block);
                    *block.layout_mut() = CanvasLayout::new(96.0, y, 520.0, height);
                    y += height + 24.0;
                }
                page.canvas_width = default_canvas_width();
                page.canvas_height = default_canvas_height().max(y + 200.0);
            }
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let assets_dir = assets_dir(path);
        fs::create_dir_all(&assets_dir)
            .with_context(|| format!("failed to create {}", assets_dir.display()))?;

        for asset in &self.assets {
            if !asset.bytes.is_empty() {
                fs::write(asset_path(&assets_dir, &asset.stored_name)?, &asset.bytes)
                    .with_context(|| format!("failed to save imported asset {}", asset.filename))?;
            }
        }

        let json = serde_json::to_vec_pretty(self)?;
        let temporary = path.with_extension("onl.tmp");
        fs::write(&temporary, json)
            .with_context(|| format!("failed to write {}", temporary.display()))?;
        fs::rename(&temporary, path)
            .with_context(|| format!("failed to replace {}", path.display()))?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let data = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
        let mut project: Self = serde_json::from_slice(&data)
            .with_context(|| format!("invalid OneNote Linux project: {}", path.display()))?;
        if !(1..=FORMAT_VERSION).contains(&project.format_version) {
            bail!(
                "unsupported project version {} (latest {})",
                project.format_version,
                FORMAT_VERSION
            );
        }
        if project.format_version == 1 {
            project.migrate_legacy_canvas();
            project.format_version = FORMAT_VERSION;
        }

        let assets_dir = assets_dir(path);
        for asset in &mut project.assets {
            asset.bytes =
                fs::read(asset_path(&assets_dir, &asset.stored_name)?).unwrap_or_default();
        }
        Ok(project)
    }

    pub fn export_markdown(&self, path: &Path) -> Result<()> {
        let export_assets_dir = assets_dir(path);
        let export_assets_name = export_assets_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("notebook.md.assets");
        if self.assets.iter().any(|asset| !asset.bytes.is_empty()) {
            fs::create_dir_all(&export_assets_dir).with_context(|| {
                format!(
                    "failed to create Markdown asset directory {}",
                    export_assets_dir.display()
                )
            })?;
            for asset in &self.assets {
                if !asset.bytes.is_empty() {
                    fs::write(
                        asset_path(&export_assets_dir, &asset.stored_name)?,
                        &asset.bytes,
                    )
                    .with_context(|| {
                        format!("failed to export imported asset {}", asset.filename)
                    })?;
                }
            }
        }

        let mut output = format!("# {}\n\n", self.name);
        for section in &self.sections {
            output.push_str(&format!("## {}\n\n", section.name));
            for page in &section.pages {
                output.push_str(&format!("### {}\n\n", page.title));
                for block in &page.blocks {
                    match block {
                        EditorBlock::Text { text, indent, .. } => {
                            output.push_str(&"  ".repeat(*indent as usize));
                            output.push_str(text);
                            output.push_str("\n\n");
                        }
                        EditorBlock::Table { rows, .. } => append_table(&mut output, rows),
                        EditorBlock::Image { asset_id, .. } => {
                            if let Some(asset) = self.asset(*asset_id) {
                                output.push_str(&format!(
                                    "![{}]({}/{})\n\n",
                                    asset.filename, export_assets_name, asset.stored_name
                                ));
                            }
                        }
                        EditorBlock::Attachment { asset_id, .. } => {
                            if let Some(asset) = self.asset(*asset_id) {
                                output.push_str(&format!(
                                    "[Attachment: {}]({}/{})\n\n",
                                    asset.filename, export_assets_name, asset.stored_name
                                ));
                            }
                        }
                        EditorBlock::Ink { stroke_count, .. } => {
                            output.push_str(&format!("_[Ink: {stroke_count} strokes]_\n\n"));
                        }
                        EditorBlock::Unsupported { description, .. } => {
                            output.push_str(&format!("_[Unsupported: {description}]_\n\n"));
                        }
                    }
                }
            }
        }
        fs::write(path, output).with_context(|| format!("failed to export {}", path.display()))?;
        Ok(())
    }
}

pub(super) fn estimated_block_height(block: &EditorBlock) -> f32 {
    match block {
        EditorBlock::Text { text, .. } => {
            (text.lines().count().max(1) as f32 * 24.0 + 28.0).max(72.0)
        }
        EditorBlock::Table { rows, .. } => (rows.len().max(1) as f32 * 34.0 + 48.0).max(96.0),
        EditorBlock::Image { layout, .. } => layout.height.max(240.0),
        EditorBlock::Attachment { .. } => 72.0,
        EditorBlock::Ink { layout, .. } => layout.height.max(120.0),
        EditorBlock::Unsupported { .. } => 64.0,
    }
}

fn append_table(output: &mut String, rows: &[Vec<String>]) {
    let columns = rows.iter().map(Vec::len).max().unwrap_or_default();
    if columns == 0 {
        return;
    }
    for column in 0..columns {
        output.push_str(if column == 0 { "| " } else { " | " });
        output.push_str(
            rows.first()
                .and_then(|row| row.get(column))
                .map(String::as_str)
                .unwrap_or(""),
        );
    }
    output.push_str(" |\n");
    output.push_str(&format!("|{}|\n", " --- |".repeat(columns)));
    for row in rows.iter().skip(1) {
        for column in 0..columns {
            output.push_str(if column == 0 { "| " } else { " | " });
            output.push_str(row.get(column).map(String::as_str).unwrap_or(""));
        }
        output.push_str(" |\n");
    }
    output.push('\n');
}

fn assets_dir(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("notebook.onl");
    path.with_file_name(format!("{file_name}.assets"))
}

fn asset_path(directory: &Path, stored_name: &str) -> Result<PathBuf> {
    let mut components = Path::new(stored_name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(name)), None) => Ok(directory.join(name)),
        _ => bail!("invalid project asset path: {stored_name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_project_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.onl");
        let project = Project::empty();

        project.save(&path).unwrap();
        let loaded = Project::load(&path).unwrap();

        assert_eq!(loaded.name, project.name);
        assert_eq!(loaded.sections.len(), 1);
    }

    #[test]
    fn legacy_fixed_width_strokes_load_without_point_widths() {
        let stroke: EditorStroke = serde_json::from_str(
            r#"{"points":[{"x":1.0,"y":2.0},{"x":3.0,"y":4.0}],"color":[0,0,0,255],"width":2.5}"#,
        )
        .unwrap();
        assert!(stroke.point_widths.is_empty());
    }

    #[test]
    fn legacy_graph_links_default_to_pre_update_sync_version() {
        let link: GraphPageLink = serde_json::from_str(
            r#"{"graph_page_id":"page-id","uploaded_at":"2026-06-22T12:00:00Z"}"#,
        )
        .unwrap();

        assert_eq!(link.sync_version, 0);
    }

    #[test]
    fn filename_sanitization_blocks_paths() {
        assert_eq!(safe_filename("../../secret.txt"), "secret.txt");
    }

    #[test]
    fn adjacent_paragraphs_share_one_editable_outline_box() {
        let mut blocks = vec![
            EditorBlock::Text {
                id: 1,
                text: "First paragraph".to_owned(),
                indent: 0,
                layout: CanvasLayout::default(),
            },
            EditorBlock::Text {
                id: 2,
                text: "Nested paragraph".to_owned(),
                indent: 1,
                layout: CanvasLayout::default(),
            },
        ];

        merge_adjacent_text_blocks(&mut blocks);

        assert_eq!(blocks.len(), 1);
        let EditorBlock::Text { text, .. } = &blocks[0] else {
            panic!("expected text block");
        };
        assert_eq!(text, "First paragraph\n  Nested paragraph");
    }

    #[test]
    fn assets_survive_save_and_load() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("assets.onl");
        let mut project = Project::empty();
        let asset_id = project.allocate_id();
        project.assets.push(Asset {
            id: asset_id,
            filename: "diagram.png".to_owned(),
            media_type: "image/png".to_owned(),
            stored_name: "00000004-diagram.png".to_owned(),
            size: 4,
            bytes: vec![1, 2, 3, 4],
        });

        project.save(&path).unwrap();
        let loaded = Project::load(&path).unwrap();

        assert_eq!(loaded.assets[0].bytes, [1, 2, 3, 4]);
    }

    #[test]
    fn markdown_export_copies_assets() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("export.md");
        let mut project = Project::empty();
        let asset_id = project.allocate_id();
        project.assets.push(Asset {
            id: asset_id,
            filename: "diagram.png".to_owned(),
            media_type: "image/png".to_owned(),
            stored_name: "00000004-diagram.png".to_owned(),
            size: 4,
            bytes: vec![1, 2, 3, 4],
        });
        let block_id = project.allocate_id();
        project.sections[0].pages[0]
            .blocks
            .push(EditorBlock::Image {
                id: block_id,
                asset_id,
                layout: CanvasLayout::default(),
            });

        project.export_markdown(&path).unwrap();

        let markdown = fs::read_to_string(&path).unwrap();
        assert!(markdown.contains("export.md.assets/00000004-diagram.png"));
        assert_eq!(
            fs::read(
                directory
                    .path()
                    .join("export.md.assets/00000004-diagram.png")
            )
            .unwrap(),
            [1, 2, 3, 4]
        );
    }

    #[test]
    fn legacy_projects_are_migrated_to_the_canvas_format() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.onl");
        let legacy = serde_json::json!({
            "format_version": 1,
            "name": "Legacy",
            "source": null,
            "sections": [{
                "id": 1,
                "name": "Section",
                "pages": [{
                    "id": 2,
                    "title": "Page",
                    "level": 0,
                    "author": null,
                    "created_at": "",
                    "updated_at": "",
                    "blocks": [{
                        "type": "text",
                        "id": 3,
                        "text": "legacy text",
                        "indent": 0
                    }]
                }]
            }],
            "assets": [],
            "next_id": 4
        });
        fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();

        let project = Project::load(&path).unwrap();

        assert_eq!(project.format_version, FORMAT_VERSION);
        assert!(project.sections[0].pages[0].blocks[0].layout().width > 100.0);
    }

    #[test]
    fn native_ink_delta_paths_are_decoded_without_collapsing_stroke_origins() {
        let ink = OneNoteInk {
            layout: OneNoteLayout {
                x: Some(2.0),
                y: Some(3.0),
                width: None,
                height: None,
            },
            loaded: true,
            stroke_count: 2,
            strokes: vec![
                libonenote::InkStroke {
                    points: vec![
                        libonenote::Point { x: 0.0, y: 0.0 },
                        libonenote::Point { x: 1270.0, y: 0.0 },
                    ],
                    pen_tip: Some(0),
                    transparency: Some(0),
                    width: 127.0,
                    height: 127.0,
                    color: Some(0x0003_0201),
                },
                libonenote::InkStroke {
                    points: vec![
                        libonenote::Point { x: 2540.0, y: 0.0 },
                        libonenote::Point { x: 0.0, y: 1270.0 },
                    ],
                    pen_tip: Some(0),
                    transparency: Some(0),
                    width: 127.0,
                    height: 127.0,
                    color: Some(0x0003_0201),
                },
            ],
            groups: Vec::new(),
        };
        let mut strokes = Vec::new();
        collect_ink_strokes(&ink, true, &mut strokes);
        let origin = normalize_strokes(&mut strokes).unwrap();

        assert_eq!(strokes[0].color, [1, 2, 3, 255]);
        assert!((origin.x - 96.0).abs() < 0.01);
        assert!((origin.y - 144.0).abs() < 0.01);
        assert!((strokes[0].points[1].x - 48.0).abs() < 0.01);
        assert!((strokes[1].points[0].x - 96.0).abs() < 0.01);
        assert!((strokes[1].points[1].x - 96.0).abs() < 0.01);
        assert!((strokes[1].points[1].y - 48.0).abs() < 0.01);
        assert!((strokes[0].width - 4.8).abs() < 0.01);
    }

    #[test]
    fn project_assets_cannot_escape_the_asset_directory() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("unsafe.onl");
        let mut project = Project::empty();
        let asset_id = project.allocate_id();
        project.assets.push(Asset {
            id: asset_id,
            filename: "unsafe.txt".to_owned(),
            media_type: "text/plain".to_owned(),
            stored_name: "../unsafe.txt".to_owned(),
            size: 4,
            bytes: b"nope".to_vec(),
        });

        assert!(project.save(&path).is_err());
        assert!(!directory.path().join("unsafe.txt").exists());
    }
}
