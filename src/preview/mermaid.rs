//! Renders Mermaid diagrams to textures on a background thread.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::panic::AssertUnwindSafe;
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{ColorImage, TextureHandle, TextureOptions};
use merman::svg::export::{NativeFontContext, NativeFontOptions, RasterOptions};
use merman::{
    MermaidEngine, OperationControl, PngRequest, RenderOutput, RenderRequest, Renderer,
    SourcePresentationConfig, SvgRequest,
};

/// Diagrams are drawn at this many pixels per point so they stay sharp.
const SCALE: f32 = 2.0;
const MAX_SIDE: usize = 8192;
const CACHE_LIMIT: usize = 64;

/// A diagram position in a document: which document, which diagram, which theme.
type Slot = (u64, usize, bool);

struct Request {
    key: u64,
    slot: Slot,
    source: String,
}

/// A finished request; `None` means it was skipped as out of date.
type Finished = (u64, Option<Result<ColorImage, String>>);

enum State {
    Pending,
    Ready(TextureHandle),
    Failed(String),
}

pub struct Diagrams {
    ctx: egui::Context,
    worker: Option<Worker>,
    cache: HashMap<u64, State>,
    last_good: HashMap<Slot, u64>,
}

impl Diagrams {
    pub fn new(ctx: egui::Context) -> Self {
        Self {
            ctx,
            worker: None,
            cache: HashMap::new(),
            last_good: HashMap::new(),
        }
    }

    /// Collects finished renders. Returns true if anything arrived.
    pub fn poll(&mut self) -> bool {
        let Some((_, results)) = &self.worker else {
            return false;
        };
        let mut any = false;
        for (key, result) in results.try_iter() {
            let Some(result) = result else {
                // Skipped: forget it, so it is rendered if the text comes back.
                self.cache.remove(&key);
                continue;
            };
            let state = match result {
                Ok(image) => State::Ready(self.ctx.load_texture(
                    format!("mermaid-{key}"),
                    image,
                    TextureOptions::LINEAR,
                )),
                Err(message) => State::Failed(message),
            };
            self.cache.insert(key, state);
            any = true;
        }
        any
    }

    pub fn show(&mut self, ui: &mut egui::Ui, doc_id: u64, ordinal: usize, source: &str) {
        let dark = ui.visuals().dark_mode;
        let mut hasher = DefaultHasher::new();
        (source, dark).hash(&mut hasher);
        let key = hasher.finish();
        let slot = (doc_id, ordinal, dark);

        if !self.cache.contains_key(&key) {
            self.evict();
            self.cache.insert(key, State::Pending);
            let (requests, _) = self.worker.get_or_insert_with(|| spawn(self.ctx.clone()));
            let _ = requests.send(Request {
                key,
                slot,
                source: source.to_owned(),
            });
        }

        match &self.cache[&key] {
            State::Ready(texture) => {
                self.last_good.insert(slot, key);
                image(ui, texture);
            }
            State::Pending => {
                if !self.previous(ui, slot) {
                    ui.weak("Rendering diagram…");
                }
            }
            State::Failed(message) => {
                let message = message.clone();
                self.previous(ui, slot);
                ui.colored_label(ui.visuals().error_fg_color, format!("Mermaid: {message}"));
            }
        }
    }

    fn previous(&self, ui: &mut egui::Ui, slot: Slot) -> bool {
        match self.last_good.get(&slot).and_then(|k| self.cache.get(k)) {
            Some(State::Ready(texture)) => {
                image(ui, texture);
                true
            }
            _ => false,
        }
    }

    fn evict(&mut self) {
        if self.cache.len() >= CACHE_LIMIT {
            let keep: Vec<u64> = self.last_good.values().copied().collect();
            self.cache
                .retain(|key, state| keep.contains(key) || matches!(state, State::Pending));
        }
    }
}

fn image(ui: &mut egui::Ui, texture: &TextureHandle) {
    let size = texture.size_vec2() / SCALE;
    let fit = (ui.available_width() / size.x).min(1.0);
    ui.add(egui::Image::from_texture((texture.id(), size * fit)));
}

type Worker = (Sender<Request>, Receiver<Finished>);

fn spawn(ctx: egui::Context) -> Worker {
    let (requests, request_rx) = channel::<Request>();
    let (result_tx, results) = channel();
    std::thread::Builder::new()
        .name("mermaid".into())
        .spawn(move || {
            // Loaded on the first diagram so that startup stays fast.
            let fonts = load_fonts();
            let mut queue: Vec<Request> = Vec::new();
            loop {
                if queue.is_empty() {
                    match request_rx.recv() {
                        Ok(request) => queue.push(request),
                        Err(_) => return,
                    }
                }
                queue.extend(request_rx.try_iter());
                let request = queue.remove(0);
                // While typing, only the newest text of a diagram matters.
                let superseded = queue.iter().any(|r| r.slot == request.slot);
                let result = (!superseded).then(|| {
                    let dark = request.slot.2;
                    let job = AssertUnwindSafe(|| render(&request.source, dark, &fonts));
                    std::panic::catch_unwind(job)
                        .unwrap_or_else(|_| Err("the renderer crashed on this diagram".into()))
                });
                if result_tx.send((request.key, result)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        })
        .expect("failed to start the diagram thread");
    (requests, results)
}

fn load_fonts() -> Result<NativeFontContext, String> {
    let mut options = NativeFontOptions::default();
    for name in [
        "Trebuchet MS.ttf",
        "Trebuchet MS Bold.ttf",
        "Arial.ttf",
        "Arial Bold.ttf",
        "Courier New.ttf",
    ] {
        let path = std::path::Path::new("/System/Library/Fonts/Supplemental").join(name);
        if path.exists() {
            options = options.with_font_file(path);
        }
    }
    options
        .load(&OperationControl::new())
        .map_err(|e| format!("could not load fonts: {e}"))
}

fn render(
    source: &str,
    dark: bool,
    fonts: &Result<NativeFontContext, String>,
) -> Result<ColorImage, String> {
    let fonts = fonts.clone()?;
    let theme = if dark { "dark" } else { "default" };
    let config = SourcePresentationConfig::from_value(serde_json::json!({ "theme": theme }));
    let renderer =
        Renderer::new().with_presentation_engine(MermaidEngine::new().with_site_config(config));
    let output = renderer
        .render(RenderRequest::png(
            source,
            OperationControl::new(),
            PngRequest {
                svg: SvgRequest::default(),
                options: RasterOptions::default()
                    .with_font_context(fonts)
                    .with_scale(SCALE),
            },
        ))
        .map_err(|e| e.to_string())?;
    let RenderOutput::Png(Some(png)) = output else {
        return Err("no diagram found".into());
    };
    let rgba = image::load_from_memory_with_format(png.bytes(), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    if size[0] > MAX_SIDE || size[1] > MAX_SIDE {
        return Err("diagram is too large to display".into());
    }
    Ok(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_common_diagrams_in_both_themes() {
        let fonts = load_fonts();
        for source in [
            "flowchart LR\n  A[Start] --> B{OK?}",
            "sequenceDiagram\n  A->>B: hi",
            "classDiagram\n  class Doc { +save() }",
            "stateDiagram-v2\n  [*] --> Clean",
        ] {
            for dark in [false, true] {
                render(source, dark, &fonts).unwrap_or_else(|e| panic!("{source}: {e}"));
            }
        }
        assert!(render("not a diagram", false, &fonts).is_err());
    }
}
