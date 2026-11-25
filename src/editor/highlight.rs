//! Markdown coloring, one line at a time.

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};

pub struct Style {
    pub font: FontId,
    pub text: Color32,
    heading: Color32,
    code: Color32,
    quote: Color32,
    marker: Color32,
    link: Color32,
    strong: Color32,
}

impl Style {
    pub fn new(dark: bool, font: FontId, text: Color32) -> Self {
        let rgb = Color32::from_rgb;
        if dark {
            Self {
                font,
                text,
                heading: rgb(121, 184, 255),
                code: rgb(230, 175, 120),
                quote: rgb(140, 160, 140),
                marker: rgb(200, 140, 220),
                link: rgb(90, 200, 200),
                strong: rgb(255, 255, 255),
            }
        } else {
            Self {
                font,
                text,
                heading: rgb(0, 80, 170),
                code: rgb(165, 70, 0),
                quote: rgb(85, 115, 85),
                marker: rgb(135, 50, 160),
                link: rgb(0, 115, 125),
                strong: rgb(0, 0, 0),
            }
        }
    }
}

/// Colors one line. `code` is true inside a code fence and on the fence lines.
pub fn line(text: &str, code: bool, style: &Style) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut push = |s: &str, color: Color32| {
        if !s.is_empty() {
            job.append(s, 0.0, TextFormat::simple(style.font.clone(), color));
        }
    };

    let trimmed = text.trim_start();
    if code {
        push(text, style.code);
    } else if trimmed.starts_with('#') {
        push(text, style.heading);
    } else if trimmed.starts_with('>') {
        push(text, style.quote);
    } else {
        let indent = text.len() - trimmed.len();
        let marker = list_marker_len(trimmed);
        push(&text[..indent], style.text);
        push(&trimmed[..marker], style.marker);
        inline(&trimmed[marker..], style, &mut push);
    }
    if job.sections.is_empty() {
        // An empty line still needs the font, to get the right height.
        job.append("", 0.0, TextFormat::simple(style.font.clone(), style.text));
    }
    job
}

fn list_marker_len(line: &str) -> usize {
    if line.starts_with("- ") || line.starts_with("* ") || line.starts_with("+ ") {
        return 2;
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && (line[digits..].starts_with(". ") || line[digits..].starts_with(") ")) {
        digits + 2
    } else {
        0
    }
}

/// Colors `code`, **strong** and [links](url) within one line.
fn inline(line: &str, style: &Style, push: &mut impl FnMut(&str, Color32)) {
    let bytes = line.as_bytes();
    let (mut plain, mut i) = (0, 0);
    // Once a closing mark is missing, stop searching for it, so that a long
    // line full of unmatched marks stays linear.
    let (mut ticks, mut stars, mut links) = (true, true, true);
    while i < bytes.len() {
        let rest = &line[i..];
        let span = match bytes[i] {
            b'`' if ticks => {
                let end = rest[1..].find('`');
                ticks = end.is_some();
                end.map(|n| (n + 2, style.code))
            }
            b'*' if stars && rest.starts_with("**") => {
                let end = rest[2..].find("**");
                stars = end.is_some();
                end.map(|n| (n + 4, style.strong))
            }
            b'[' if links => {
                let end = rest
                    .find("](")
                    .and_then(|mid| rest[mid..].find(')').map(|end| mid + end + 1));
                links = end.is_some();
                end.map(|n| (n, style.link))
            }
            _ => None,
        };
        match span {
            Some((len, color)) => {
                push(&line[plain..i], style.text);
                push(&line[i..i + len], color);
                i += len;
                plain = i;
            }
            // Step over the whole character so slicing stays on boundaries.
            None => i += rest.chars().next().map_or(1, char::len_utf8),
        }
    }
    push(&line[plain..], style.text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coloring_keeps_every_character() {
        let style = Style::new(true, FontId::monospace(12.0), Color32::WHITE);
        for source in [
            "# H é",
            "> q",
            "- item `c` **b** [l](u) é",
            "12. x",
            "`open [x](",
            "** é [",
            "",
        ] {
            for code in [false, true] {
                let job = line(source, code, &style);
                assert_eq!(job.text, source);
                let covered: usize = job
                    .sections
                    .iter()
                    .map(|s| s.byte_range.end.0 - s.byte_range.start.0)
                    .sum();
                assert_eq!(covered, source.len());
            }
        }
    }
}
