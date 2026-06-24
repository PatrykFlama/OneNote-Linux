use super::{CanvasLayout, EditorBlock, EditorPage};
use libonenote::{
    Content, GraphPageExport, GraphWriteOptions, Ink, Layout, Notebook, NotebookEntry, Outline,
    OutlineElement, OutlineItem, Page, PageBlock, Paragraph, Section, Table, TableCell, TableRow,
    TextAlignment, TextStyle,
};

const HALF_INCH_TO_PIXELS: f32 = 48.0;
pub const GRAPH_SYNC_VERSION: u32 = 1;
const CONTENT_DATA_ID: &str = "onenote-linux-content";

pub struct GraphSyncPage {
    pub export: GraphPageExport,
    pub replacement_html: String,
}

impl EditorPage {
    pub fn to_graph_page(&self, section_name: &str) -> libonenote::Result<GraphSyncPage> {
        let page = Page {
            id: self.id.to_string(),
            title: self.title.clone(),
            level: self.level,
            author: self.author.clone(),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            height: Some(self.canvas_height / HALF_INCH_TO_PIXELS),
            blocks: self.blocks.iter().map(map_block).collect(),
        };
        let notebook = Notebook {
            name: "OneNote Linux sync".to_owned(),
            color: None,
            entries: vec![NotebookEntry::Section(Section {
                name: section_name.to_owned(),
                color: None,
                pages: vec![page],
            })],
        };
        let mut export = notebook
            .to_graph_export(GraphWriteOptions::with_placeholders())?
            .pages
            .into_iter()
            .next()
            .expect("single-page Graph export");
        let inner = body_inner(&export.html).unwrap_or_default();
        let replacement_html = format!("<div data-id=\"{CONTENT_DATA_ID}\">{inner}</div>");
        let root = format!(
            "<div data-id=\"onenote-linux-root\" style=\"position:absolute;left:0px;top:0px;\
             width:{}px;height:{}px\">{replacement_html}</div>",
            self.canvas_width, self.canvas_height
        );
        export.html = replace_body_inner(&export.html, &root);
        Ok(GraphSyncPage {
            export,
            replacement_html,
        })
    }
}

fn body_inner(html: &str) -> Option<&str> {
    let body = html.find("<body")?;
    let start = body + html[body..].find('>')? + 1;
    let end = html.rfind("</body>")?;
    (start <= end).then_some(&html[start..end])
}

fn replace_body_inner(html: &str, replacement: &str) -> String {
    let Some(body) = html.find("<body") else {
        return html.to_owned();
    };
    let Some(open_end) = html[body..].find('>').map(|offset| body + offset + 1) else {
        return html.to_owned();
    };
    let Some(close_start) = html.rfind("</body>") else {
        return html.to_owned();
    };
    format!(
        "{}{}{}",
        &html[..open_end],
        replacement,
        &html[close_start..]
    )
}

fn map_block(block: &EditorBlock) -> PageBlock {
    match block {
        EditorBlock::Text {
            text,
            indent,
            layout,
            ..
        } => PageBlock::Outline(Outline {
            level: 0,
            layout: map_layout(*layout),
            items: vec![OutlineItem::Element(OutlineElement {
                level: *indent,
                lists: Vec::new(),
                content: vec![Content::Paragraph(Paragraph {
                    text: text.clone(),
                    style: TextStyle::default(),
                    runs: Vec::new(),
                    alignment: TextAlignment::Left,
                    space_before: 0.0,
                    space_after: 0.0,
                })],
                children: Vec::new(),
            })],
        }),
        EditorBlock::Table { rows, layout, .. } => {
            let columns = rows.iter().map(Vec::len).max().unwrap_or_default();
            PageBlock::Outline(Outline {
                level: 0,
                layout: map_layout(*layout),
                items: vec![OutlineItem::Element(OutlineElement {
                    level: 0,
                    lists: Vec::new(),
                    content: vec![Content::Table(Table {
                        rows: rows.len() as u32,
                        columns: columns as u32,
                        column_widths: Vec::new(),
                        borders_visible: true,
                        content: rows
                            .iter()
                            .map(|row| TableRow {
                                cells: (0..columns)
                                    .map(|column| TableCell {
                                        background: None,
                                        content: vec![text_element(
                                            row.get(column).map(String::as_str).unwrap_or(""),
                                        )],
                                    })
                                    .collect(),
                            })
                            .collect(),
                    })],
                    children: Vec::new(),
                })],
            })
        }
        EditorBlock::Ink {
            stroke_count,
            layout,
            ..
        } => PageBlock::Ink(Ink {
            layout: map_layout(*layout),
            loaded: true,
            stroke_count: *stroke_count,
            strokes: Vec::new(),
            groups: Vec::new(),
        }),
        EditorBlock::Image { .. }
        | EditorBlock::Attachment { .. }
        | EditorBlock::Unsupported { .. } => PageBlock::Unknown,
    }
}

fn text_element(text: &str) -> OutlineElement {
    OutlineElement {
        level: 0,
        lists: Vec::new(),
        content: vec![Content::Paragraph(Paragraph {
            text: text.to_owned(),
            style: TextStyle::default(),
            runs: Vec::new(),
            alignment: TextAlignment::Left,
            space_before: 0.0,
            space_after: 0.0,
        })],
        children: Vec::new(),
    }
}

fn map_layout(layout: CanvasLayout) -> Layout {
    Layout {
        x: Some(layout.x / HALF_INCH_TO_PIXELS),
        y: Some(layout.y / HALF_INCH_TO_PIXELS),
        width: Some(layout.width / HALF_INCH_TO_PIXELS),
        height: Some(layout.height / HALF_INCH_TO_PIXELS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::CanvasPoint;
    use crate::project::EditorStroke;

    #[test]
    fn editor_page_serializes_to_graph_with_explicit_ink_warning() {
        let page = EditorPage {
            id: 1,
            title: "Page".to_owned(),
            level: 0,
            author: None,
            created_at: String::new(),
            updated_at: String::new(),
            canvas_width: 1600.0,
            canvas_height: 1200.0,
            blocks: vec![EditorBlock::Ink {
                id: 2,
                stroke_count: 1,
                strokes: vec![EditorStroke {
                    points: vec![
                        CanvasPoint { x: 0.0, y: 0.0 },
                        CanvasPoint { x: 1.0, y: 1.0 },
                    ],
                    color: [0, 0, 0, 255],
                    width: 2.0,
                    point_widths: Vec::new(),
                }],
                layout: CanvasLayout::default(),
            }],
        };

        let page = page.to_graph_page("Section").unwrap();
        assert!(page.export.html.contains("[Ink: 1 strokes]"));
        assert!(
            page.export
                .html
                .contains("data-id=\"onenote-linux-content\"")
        );
        assert_eq!(page.export.warnings.len(), 1);
        assert!(page.replacement_html.starts_with("<div data-id="));
    }
}
