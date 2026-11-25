//! Headless timing of the real app UI. Run with:
//! `cargo test --release perf -- --ignored --nocapture`

use std::time::{Duration, Instant};

use crate::app::{App, View};

fn sample(bytes: usize) -> String {
    let mut s = String::new();
    let mut i = 0;
    while s.len() < bytes {
        s.push_str(&format!(
            "## Heading {i}\n\nSome **bold** text with a [link](http://x) and `code` that is long enough to wrap around once or twice in a normal editor window, line {i}.\n\n- item one\n- item two\n\n```rust\nfn f{i}() {{}}\n```\n\n"
        ));
        i += 1;
    }
    s
}

fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> f64 {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1200.0, 800.0),
        )),
        events,
        focused: true,
        ..Default::default()
    };
    let start = Instant::now();
    let mut output = ctx.run_ui(input, |ui| app.frame(ui));
    output.textures_delta.clear();
    let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
    start.elapsed().as_secs_f64() * 1e3
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

#[test]
#[ignore = "timing run, release builds only"]
fn perf() {
    for kb in [100, 1000, 5000] {
        for view in [View::Editor, View::Split, View::Preview] {
            let ctx = egui::Context::default();
            let mut app = App::with_context(&ctx, view, egui::ThemePreference::Light);
            app.active_state().0.text = sample(kb * 1000);
            app.active_state().0.mark_changed();

            let first = frame(&ctx, &mut app, vec![]);
            let doc_id = app.active_state().0.id;
            ctx.memory_mut(|m| m.request_focus(egui::Id::unique(("editor", doc_id))));
            let wait = |app: &mut App| {
                let start = Instant::now();
                while view != View::Editor && !app.active_state().1 {
                    std::thread::sleep(Duration::from_millis(1));
                    frame(&ctx, app, vec![]);
                    assert!(start.elapsed() < Duration::from_secs(30));
                }
                start.elapsed().as_secs_f64() * 1e3
            };
            wait(&mut app);
            for _ in 0..3 {
                frame(&ctx, &mut app, vec![]);
            }

            let (mut keys, mut previews) = (vec![], vec![]);
            for _ in 0..15 {
                if view != View::Preview {
                    let started = Instant::now();
                    keys.push(frame(&ctx, &mut app, vec![egui::Event::Text("a".into())]));
                    if view == View::Split {
                        wait(&mut app);
                        previews.push(started.elapsed().as_secs_f64() * 1e3);
                    }
                }
            }
            let idle = median((0..15).map(|_| frame(&ctx, &mut app, vec![])).collect());
            // The timings only mean something if the typing really happened.
            let typed = if view == View::Preview { 0 } else { 15 };
            assert_eq!(
                app.active_state().0.text.len(),
                sample(kb * 1000).len() + typed
            );
            let show = |v: Vec<f64>| {
                if v.is_empty() {
                    "-".to_owned()
                } else {
                    format!("{:.1}", median(v))
                }
            };
            println!(
                "{kb:>5} KB {view:?}: first {first:.0} ms, keystroke {} ms, preview updated {} ms, redraw {idle:.1} ms",
                show(keys),
                show(previews)
            );
        }
    }
}
