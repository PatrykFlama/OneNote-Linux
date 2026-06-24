use crate::project::{EditorBlock, EditorPage};

#[derive(Clone)]
pub(super) struct PageSnapshot {
    blocks: Vec<EditorBlock>,
    canvas_width: f32,
    canvas_height: f32,
}

impl PageSnapshot {
    pub(super) fn capture(page: &EditorPage) -> Self {
        Self {
            blocks: page.blocks.clone(),
            canvas_width: page.canvas_width,
            canvas_height: page.canvas_height,
        }
    }

    pub(super) fn restore(self, page: &mut EditorPage) {
        page.blocks = self.blocks;
        page.canvas_width = self.canvas_width;
        page.canvas_height = self.canvas_height;
    }
}

pub(super) fn push_history(stack: &mut Vec<PageSnapshot>, snapshot: PageSnapshot) {
    const HISTORY_LIMIT: usize = 64;
    if stack.len() == HISTORY_LIMIT {
        stack.remove(0);
    }
    stack.push(snapshot);
}

pub(super) fn commit_pending_history(
    pending: &mut Option<PageSnapshot>,
    undo_stack: &mut Vec<PageSnapshot>,
    redo_stack: &mut Vec<PageSnapshot>,
) {
    if let Some(snapshot) = pending.take() {
        push_history(undo_stack, snapshot);
        redo_stack.clear();
    }
}
