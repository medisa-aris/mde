//! Draws only the blocks that are on screen.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

use super::blocks::{Block, Kind};
use super::mermaid::Diagrams;

const MARGIN: f32 = 16.0;
const SPACING: f32 = 10.0;
const ESTIMATED_LINE: f32 = 20.0;

/// Per-document preview state.
#[derive(Default)]
pub struct Preview {
    blocks: Arc<[Block]>,
    /// For each block, how many diagrams come before it.
    ordinals: Vec<usize>,
    heights: Vec<f32>,
    /// Measured heights by block content.
    measured: HashMap<u64, f32>,
    width: f32,
    /// Revision of the text the blocks came from.
    pub revision: Option<u64>,
}

pub struct PreviewOutput {
    /// Source character shown at the top of the visible area.
    pub top_char: f32,
    pub hovered: bool,
}

impl Preview {
    pub fn set_blocks(&mut self, blocks: Arc<[Block]>, revision: u64) {
        let mut diagrams = 0;
        self.ordinals = blocks
            .iter()
            .map(|b| {
                let ordinal = diagrams;
                diagrams += usize::from(matches!(b.kind, Kind::Mermaid(_)));
                ordinal
            })
            .collect();
        self.heights = blocks
            .iter()
            .map(|b| {
                self.measured
                    .get(&b.hash)
                    .copied()
                    .unwrap_or(b.lines as f32 * ESTIMATED_LINE + SPACING)
            })
            .collect();
        if self.measured.len() > blocks.len() * 4 + 256 {
            self.measured.clear();
        }
        self.blocks = blocks;
        self.revision = Some(revision);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        doc_id: u64,
        base_dir: Option<&Path>,
        cache: &mut CommonMarkCache,
        diagrams: &mut Diagrams,
        scroll_to_char: Option<f32>,
    ) -> PreviewOutput {
        let hovered = ui.rect_contains_pointer(ui.max_rect());
        let mut top_char = 0.0;
        let scheme = base_dir.map(|dir| format!("file://{}/", dir.display()));

        let mut area = egui::ScrollArea::vertical()
            .id_salt(("preview", doc_id))
            .auto_shrink(false);
        if let Some(target) = scroll_to_char {
            area = area.vertical_scroll_offset(self.offset_of_char(target));
        }

        area.show_viewport(ui, |ui, viewport| {
            let width = (ui.available_width() - 2.0 * MARGIN).max(50.0);
            if (width - self.width).abs() > 1.0 {
                self.width = width;
                self.measured.clear();
            }
            let origin = ui.min_rect().min;
            top_char = self.char_at_offset(viewport.top());

            let mut y = 0.0;
            let mut changed = false;
            for (i, block) in self.blocks.iter().enumerate() {
                let height = self.heights[i];
                if y > viewport.bottom() {
                    break;
                }
                if y + height >= viewport.top() {
                    let rect = egui::Rect::from_min_size(
                        origin + egui::vec2(MARGIN, y),
                        egui::vec2(width, f32::INFINITY),
                    );
                    let builder = egui::UiBuilder::new()
                        .id_salt(("block", i))
                        .max_rect(rect)
                        .layout(egui::Layout::top_down(egui::Align::Min));
                    let mut child = ui.new_child(builder);
                    match &block.kind {
                        Kind::Markdown(source) => {
                            let mut viewer =
                                CommonMarkViewer::new().syntax_theme_light("InspiredGitHub");
                            if let Some(scheme) = &scheme {
                                viewer = viewer.default_implicit_uri_scheme(scheme.clone());
                            }
                            viewer.show(&mut child, cache, source);
                        }
                        Kind::Mermaid(source) => {
                            diagrams.show(&mut child, doc_id, self.ordinals[i], source);
                        }
                    }
                    let actual = child.min_rect().height() + SPACING;
                    if (actual - height).abs() > 0.5 {
                        self.heights[i] = actual;
                        changed = true;
                    }
                    self.measured.insert(block.hash, actual);
                }
                y += self.heights[i];
            }

            let total: f32 = self.heights.iter().sum();
            ui.set_height(total + MARGIN);
            if changed {
                ui.ctx().request_repaint();
            }
        });

        PreviewOutput { top_char, hovered }
    }

    fn offset_of_char(&self, target: f32) -> f32 {
        let mut y = 0.0;
        for (block, height) in self.blocks.iter().zip(&self.heights) {
            if target < block.chars.end as f32 {
                let len = block.chars.len().max(1) as f32;
                let part = ((target - block.chars.start as f32) / len).clamp(0.0, 1.0);
                return y + part * height;
            }
            y += height;
        }
        y
    }

    fn char_at_offset(&self, offset: f32) -> f32 {
        let mut y = 0.0;
        for (block, height) in self.blocks.iter().zip(&self.heights) {
            if offset < y + height {
                let part = ((offset - y) / height).clamp(0.0, 1.0);
                return block.chars.start as f32 + part * block.chars.len() as f32;
            }
            y += height;
        }
        self.blocks.last().map_or(0.0, |b| b.chars.end as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_positions_map_both_ways() {
        let mut preview = Preview::default();
        let blocks = super::super::blocks::split("# a\n\nsecond paragraph\n\nthird\n");
        preview.set_blocks(blocks.into(), 1);
        for target in [0.0, 5.0, 12.0, 24.0] {
            let back = preview.char_at_offset(preview.offset_of_char(target));
            assert!((back - target).abs() <= 1.0, "{target} -> {back}");
        }
    }
}
