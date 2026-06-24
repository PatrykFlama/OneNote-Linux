use super::{EditorBlock, GraphSyncMetadata, Project};
use anyhow::{Result, bail};
use libonenote::{
    Content, Document, Notebook, NotebookEntry, OutlineElement, OutlineItem, PageBlock, Paragraph,
};

pub(crate) struct NativeEditPlan {
    titles: Vec<TitleEdit>,
    paragraphs: Vec<TextEdit>,
}

struct TitleEdit {
    section: usize,
    page: usize,
    old: String,
    new: String,
}

struct TextEdit {
    old: String,
    new: String,
}

impl Project {
    pub(crate) fn native_edit_plan(&self, document: &Document) -> Result<NativeEditPlan> {
        let baseline = Self::import(document);
        let mut normalized = self.clone();
        let mut titles = Vec::new();
        let mut paragraphs = Vec::new();

        if self.sections.len() != baseline.sections.len() {
            bail!(
                "This notebook has {} section(s), but the native source has {}. Native writing \
                 cannot yet add or delete sections. Save as .onl to preserve these edits.",
                self.sections.len(),
                baseline.sections.len()
            );
        }
        for section_index in 0..baseline.sections.len() {
            let baseline_section = &baseline.sections[section_index];
            let section = &self.sections[section_index];
            let normalized_section = &mut normalized.sections[section_index];
            if section.pages.len() != baseline_section.pages.len() {
                bail!(
                    "Section “{}” has {} page(s), but the native source has {}. Native writing \
                     cannot yet add or delete pages. Save as .onl to preserve these edits.",
                    section.name,
                    section.pages.len(),
                    baseline_section.pages.len()
                );
            }
            for page_index in 0..baseline_section.pages.len() {
                let baseline_page = &baseline_section.pages[page_index];
                let page = &section.pages[page_index];
                let normalized_page = &mut normalized_section.pages[page_index];

                if page.title != baseline_page.title {
                    require_unique_native_text(document.notebook(), &baseline_page.title)?;
                    titles.push(TitleEdit {
                        section: section_index,
                        page: page_index,
                        old: baseline_page.title.clone(),
                        new: page.title.clone(),
                    });
                    normalized_page.title.clone_from(&baseline_page.title);
                }

                if page.blocks.len() != baseline_page.blocks.len() {
                    bail!(
                        "Page “{}” in section “{}” has {} object(s), but the native source has {}. \
                         Native writing cannot yet add or delete page objects, including new ink, \
                         text boxes, and tables. Save as .onl to preserve these edits.",
                        page.title,
                        section.name,
                        page.blocks.len(),
                        baseline_page.blocks.len()
                    );
                }
                for block_index in 0..baseline_page.blocks.len() {
                    collect_block_edits(
                        &baseline_page.blocks[block_index],
                        &page.blocks[block_index],
                        &mut normalized_page.blocks[block_index],
                        document.notebook(),
                        &mut paragraphs,
                    )?;
                }
            }
        }

        normalized.graph_sync = GraphSyncMetadata::default();
        let mut baseline = baseline;
        baseline.graph_sync = GraphSyncMetadata::default();
        if serde_json::to_value(&normalized)? != serde_json::to_value(&baseline)? {
            bail!(
                "native saving currently supports only page-title and complete-paragraph text \
                 changes that fit existing native allocations; save other edits as an .onl \
                 working copy"
            );
        }

        Ok(NativeEditPlan { titles, paragraphs })
    }
}

impl NativeEditPlan {
    pub(crate) fn is_empty(&self) -> bool {
        self.titles.is_empty() && self.paragraphs.is_empty()
    }

    pub(crate) fn apply(self, notebook: &mut Notebook) -> Result<()> {
        for edit in self.titles {
            let section = section_mut_by_index(&mut notebook.entries, edit.section)
                .ok_or_else(|| anyhow::anyhow!("native section mapping changed"))?;
            let page = section
                .pages
                .get_mut(edit.page)
                .ok_or_else(|| anyhow::anyhow!("native page mapping changed"))?;
            if page.title != edit.old {
                bail!("native page title changed since import");
            }
            page.title = edit.new;
        }
        for edit in self.paragraphs {
            let mut replacements = 0;
            replace_notebook_paragraphs(notebook, &edit.old, &edit.new, &mut replacements)?;
            if replacements != 1 {
                bail!("native paragraph mapping changed since import");
            }
        }
        Ok(())
    }
}

fn collect_block_edits(
    baseline: &EditorBlock,
    current: &EditorBlock,
    normalized: &mut EditorBlock,
    notebook: &Notebook,
    edits: &mut Vec<TextEdit>,
) -> Result<()> {
    match (baseline, current, normalized) {
        (
            EditorBlock::Text { text: baseline, .. },
            EditorBlock::Text { text: current, .. },
            EditorBlock::Text {
                text: normalized, ..
            },
        ) if baseline != current => {
            require_exact_native_paragraph(notebook, baseline)?;
            edits.push(TextEdit {
                old: baseline.clone(),
                new: current.clone(),
            });
            normalized.clone_from(baseline);
        }
        (
            EditorBlock::Table { rows: baseline, .. },
            EditorBlock::Table { rows: current, .. },
            EditorBlock::Table {
                rows: normalized, ..
            },
        ) => {
            if current.len() != baseline.len()
                || current
                    .iter()
                    .zip(baseline)
                    .any(|(row, baseline)| row.len() != baseline.len())
            {
                bail!("native saving does not support changing table dimensions");
            }
            for row in 0..baseline.len() {
                for column in 0..baseline[row].len() {
                    if current[row][column] != baseline[row][column] {
                        require_exact_native_paragraph(notebook, &baseline[row][column])?;
                        edits.push(TextEdit {
                            old: baseline[row][column].clone(),
                            new: current[row][column].clone(),
                        });
                        normalized[row][column].clone_from(&baseline[row][column]);
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn require_exact_native_paragraph(notebook: &Notebook, text: &str) -> Result<()> {
    if text.is_empty() || count_exact_paragraphs(notebook, text) != 1 {
        bail!("the edited text does not map unambiguously to one original OneNote paragraph");
    }
    require_unique_native_text(notebook, text)
}

fn require_unique_native_text(notebook: &Notebook, text: &str) -> Result<()> {
    if text.is_empty() || count_text_occurrences(notebook, text) != 1 {
        bail!("the original text is not unique enough for a safe in-place native update");
    }
    Ok(())
}

fn count_text_occurrences(notebook: &Notebook, text: &str) -> usize {
    notebook
        .pages()
        .map(|page| {
            page.title.matches(text).count()
                + page
                    .blocks
                    .iter()
                    .map(|block| count_block_text_occurrences(block, text))
                    .sum::<usize>()
        })
        .sum()
}

fn count_block_text_occurrences(block: &PageBlock, text: &str) -> usize {
    match block {
        PageBlock::Outline(outline) => outline
            .items
            .iter()
            .map(|item| count_item_text_occurrences(item, text))
            .sum(),
        PageBlock::Image(image) => image
            .alt_text
            .as_deref()
            .map_or(0, |alt| alt.matches(text).count()),
        PageBlock::Attachment(_) | PageBlock::Ink(_) | PageBlock::Unknown => 0,
    }
}

fn count_item_text_occurrences(item: &OutlineItem, text: &str) -> usize {
    match item {
        OutlineItem::Element(element) => count_element_text_occurrences(element, text),
        OutlineItem::Group(group) => group
            .items
            .iter()
            .map(|item| count_item_text_occurrences(item, text))
            .sum(),
    }
}

fn count_element_text_occurrences(element: &OutlineElement, text: &str) -> usize {
    let direct = element
        .content
        .iter()
        .map(|content| match content {
            Content::Paragraph(paragraph) => paragraph.text.matches(text).count(),
            Content::Table(table) => table
                .content
                .iter()
                .flat_map(|row| &row.cells)
                .flat_map(|cell| &cell.content)
                .map(|element| count_element_text_occurrences(element, text))
                .sum(),
            Content::Image(image) => image
                .alt_text
                .as_deref()
                .map_or(0, |alt| alt.matches(text).count()),
            Content::Attachment(_) | Content::Ink(_) | Content::Unknown => 0,
        })
        .sum::<usize>();
    direct
        + element
            .children
            .iter()
            .map(|item| count_item_text_occurrences(item, text))
            .sum::<usize>()
}

fn count_exact_paragraphs(notebook: &Notebook, text: &str) -> usize {
    notebook
        .pages()
        .flat_map(|page| &page.blocks)
        .map(|block| count_exact_block_paragraphs(block, text))
        .sum()
}

fn count_exact_block_paragraphs(block: &PageBlock, text: &str) -> usize {
    match block {
        PageBlock::Outline(outline) => outline
            .items
            .iter()
            .map(|item| count_exact_item_paragraphs(item, text))
            .sum(),
        PageBlock::Image(_) | PageBlock::Attachment(_) | PageBlock::Ink(_) | PageBlock::Unknown => {
            0
        }
    }
}

fn count_exact_item_paragraphs(item: &OutlineItem, text: &str) -> usize {
    match item {
        OutlineItem::Element(element) => count_exact_element_paragraphs(element, text),
        OutlineItem::Group(group) => group
            .items
            .iter()
            .map(|item| count_exact_item_paragraphs(item, text))
            .sum(),
    }
}

fn count_exact_element_paragraphs(element: &OutlineElement, text: &str) -> usize {
    let direct = element
        .content
        .iter()
        .map(|content| match content {
            Content::Paragraph(paragraph) => usize::from(paragraph.text == text),
            Content::Table(table) => table
                .content
                .iter()
                .flat_map(|row| &row.cells)
                .flat_map(|cell| &cell.content)
                .map(|element| count_exact_element_paragraphs(element, text))
                .sum(),
            Content::Image(_) | Content::Attachment(_) | Content::Ink(_) | Content::Unknown => 0,
        })
        .sum::<usize>();
    direct
        + element
            .children
            .iter()
            .map(|item| count_exact_item_paragraphs(item, text))
            .sum::<usize>()
}

fn section_mut_by_index(
    entries: &mut [NotebookEntry],
    target: usize,
) -> Option<&mut libonenote::Section> {
    fn find<'a>(
        entries: &'a mut [NotebookEntry],
        target: usize,
        index: &mut usize,
    ) -> Option<&'a mut libonenote::Section> {
        for entry in entries {
            match entry {
                NotebookEntry::Section(section) => {
                    if *index == target {
                        return Some(section);
                    }
                    *index += 1;
                }
                NotebookEntry::SectionGroup(group) => {
                    if let Some(section) = find(&mut group.entries, target, index) {
                        return Some(section);
                    }
                }
            }
        }
        None
    }

    find(entries, target, &mut 0)
}

fn replace_notebook_paragraphs(
    notebook: &mut Notebook,
    old: &str,
    new: &str,
    replacements: &mut usize,
) -> Result<()> {
    for entry in &mut notebook.entries {
        replace_entry_paragraphs(entry, old, new, replacements)?;
    }
    Ok(())
}

fn replace_entry_paragraphs(
    entry: &mut NotebookEntry,
    old: &str,
    new: &str,
    replacements: &mut usize,
) -> Result<()> {
    match entry {
        NotebookEntry::Section(section) => {
            for page in &mut section.pages {
                for block in &mut page.blocks {
                    if let PageBlock::Outline(outline) = block {
                        for item in &mut outline.items {
                            replace_item_paragraphs(item, old, new, replacements)?;
                        }
                    }
                }
            }
        }
        NotebookEntry::SectionGroup(group) => {
            for entry in &mut group.entries {
                replace_entry_paragraphs(entry, old, new, replacements)?;
            }
        }
    }
    Ok(())
}

fn replace_item_paragraphs(
    item: &mut OutlineItem,
    old: &str,
    new: &str,
    replacements: &mut usize,
) -> Result<()> {
    match item {
        OutlineItem::Element(element) => {
            replace_element_paragraphs(element, old, new, replacements)
        }
        OutlineItem::Group(group) => {
            for item in &mut group.items {
                replace_item_paragraphs(item, old, new, replacements)?;
            }
            Ok(())
        }
    }
}

fn replace_element_paragraphs(
    element: &mut OutlineElement,
    old: &str,
    new: &str,
    replacements: &mut usize,
) -> Result<()> {
    for content in &mut element.content {
        match content {
            Content::Paragraph(paragraph) if paragraph.text == old => {
                replace_paragraph_text(paragraph, new)?;
                *replacements += 1;
            }
            Content::Table(table) => {
                for row in &mut table.content {
                    for cell in &mut row.cells {
                        for element in &mut cell.content {
                            replace_element_paragraphs(element, old, new, replacements)?;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    for child in &mut element.children {
        replace_item_paragraphs(child, old, new, replacements)?;
    }
    Ok(())
}

fn replace_paragraph_text(paragraph: &mut Paragraph, new: &str) -> Result<()> {
    let old_units = paragraph.text.encode_utf16().count();
    let new_units = new.encode_utf16().collect::<Vec<_>>();
    if new_units.len() > old_units {
        bail!("native text replacement does not fit the existing property allocation");
    }
    if !paragraph.runs.is_empty()
        && paragraph
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>()
            != paragraph.text
    {
        bail!("native paragraph formatting cannot be mapped safely");
    }

    let fixed_run_units = paragraph
        .runs
        .iter()
        .take(paragraph.runs.len().saturating_sub(1))
        .map(|run| run.text.encode_utf16().count())
        .sum::<usize>();
    if new_units.len() < fixed_run_units {
        bail!("shortening this paragraph would cross a formatting-run boundary");
    }

    let run_count = paragraph.runs.len();
    let mut offset = 0;
    for (index, run) in paragraph.runs.iter_mut().enumerate() {
        let length = if index + 1 == run_count {
            new_units.len() - offset
        } else {
            run.text.encode_utf16().count()
        };
        run.text = String::from_utf16(&new_units[offset..offset + length])
            .map_err(|_| anyhow::anyhow!("replacement splits a UTF-16 formatting boundary"))?;
        offset += length;
    }
    paragraph.text = new.to_owned();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use libonenote::{
        Layout, Outline, Page, Paragraph, Section, TextAlignment, TextRun, TextStyle,
    };

    fn document(text: &str) -> Document {
        let mut document = Document::new("Notebook");
        document.edit(|notebook| {
            notebook.entries.push(NotebookEntry::Section(Section {
                name: "Section".to_owned(),
                color: None,
                pages: vec![Page {
                    id: "page".to_owned(),
                    title: "Page".to_owned(),
                    level: 0,
                    author: None,
                    created_at: String::new(),
                    updated_at: String::new(),
                    height: None,
                    blocks: vec![PageBlock::Outline(Outline {
                        level: 1,
                        layout: Layout::default(),
                        items: vec![OutlineItem::Element(OutlineElement {
                            level: 1,
                            lists: Vec::new(),
                            content: vec![Content::Paragraph(Paragraph {
                                text: text.to_owned(),
                                style: TextStyle::default(),
                                runs: vec![TextRun {
                                    text: text.to_owned(),
                                    style: TextStyle::default(),
                                }],
                                alignment: TextAlignment::Left,
                                space_before: 0.0,
                                space_after: 0.0,
                            })],
                            children: Vec::new(),
                        })],
                    })],
                }],
            }));
        });
        document
    }

    #[test]
    fn maps_unique_same_size_text_back_to_native_paragraph() {
        let mut document = document("Code");
        let mut project = Project::import(&document);
        let EditorBlock::Text { text, .. } = &mut project.sections[0].pages[0].blocks[0] else {
            unreachable!()
        };
        *text = "Data".to_owned();

        let plan = project.native_edit_plan(&document).unwrap();
        document.edit(|notebook| plan.apply(notebook)).unwrap();

        let PageBlock::Outline(outline) =
            &document.notebook().sections().next().unwrap().pages[0].blocks[0]
        else {
            unreachable!()
        };
        let OutlineItem::Element(element) = &outline.items[0] else {
            unreachable!()
        };
        let Content::Paragraph(paragraph) = &element.content[0] else {
            unreachable!()
        };
        assert_eq!(paragraph.text, "Data");
    }

    #[test]
    fn rejects_layout_changes() {
        let document = document("Code");
        let mut project = Project::import(&document);
        project.sections[0].pages[0].blocks[0].layout_mut().x += 10.0;
        assert!(project.native_edit_plan(&document).is_err());
    }

    #[test]
    fn reports_the_page_when_native_object_count_changes() {
        let document = document("Code");
        let mut project = Project::import(&document);
        let block_id = project.allocate_id();
        project.sections[0].pages[0].blocks.push(EditorBlock::Text {
            id: block_id,
            text: "New box".to_owned(),
            indent: 0,
            layout: Default::default(),
        });

        let error = match project.native_edit_plan(&document) {
            Ok(_) => panic!("structural native edit was accepted"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("Page “Page” in section “Section”"));
        assert!(error.contains("has 2 object(s), but the native source has 1"));
        assert!(error.contains("Save as .onl"));
    }

    #[test]
    fn rejects_editor_boxes_merged_from_multiple_native_paragraphs() {
        let mut document = document("One");
        document.edit(|notebook| {
            let NotebookEntry::Section(section) = &mut notebook.entries[0] else {
                unreachable!()
            };
            let PageBlock::Outline(outline) = &mut section.pages[0].blocks[0] else {
                unreachable!()
            };
            outline.items.push(OutlineItem::Element(OutlineElement {
                level: 1,
                lists: Vec::new(),
                content: vec![Content::Paragraph(Paragraph {
                    text: "Two".to_owned(),
                    style: TextStyle::default(),
                    runs: vec![TextRun {
                        text: "Two".to_owned(),
                        style: TextStyle::default(),
                    }],
                    alignment: TextAlignment::Left,
                    space_before: 0.0,
                    space_after: 0.0,
                })],
                children: Vec::new(),
            }));
        });
        let mut project = Project::import(&document);
        let EditorBlock::Text { text, .. } = &mut project.sections[0].pages[0].blocks[0] else {
            unreachable!()
        };
        text.push('!');

        assert!(project.native_edit_plan(&document).is_err());
    }

    #[test]
    fn maps_shorter_text_when_it_stays_inside_formatting_boundaries() {
        let mut document = document("Zażółć");
        let mut project = Project::import(&document);
        let EditorBlock::Text { text, .. } = &mut project.sections[0].pages[0].blocks[0] else {
            unreachable!()
        };
        *text = "Żółć".to_owned();

        let plan = project.native_edit_plan(&document).unwrap();
        document.edit(|notebook| plan.apply(notebook)).unwrap();

        let PageBlock::Outline(outline) =
            &document.notebook().sections().next().unwrap().pages[0].blocks[0]
        else {
            unreachable!()
        };
        let OutlineItem::Element(element) = &outline.items[0] else {
            unreachable!()
        };
        let Content::Paragraph(paragraph) = &element.content[0] else {
            unreachable!()
        };
        assert_eq!(paragraph.text, "Żółć");
        assert_eq!(paragraph.runs[0].text, "Żółć");
    }
}
