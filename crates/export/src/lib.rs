//! pdfcraft-export — Export a PDF ▸ Word, HTML, RTF, Markdown (L4).
//!
//! The engine reduces each page to [`Page`]: paragraphs (text, box, size, bold/italic) and
//! images (encoded bytes and box), in reading order. The writers turn that into a flowing
//! document: headings are the paragraphs set larger than the body text, and a block that
//! continues another on its line (an italic word, a link) is a run of that block's paragraph.
//! Word keeps pages with
//! rotated text, or text overlapped by artwork or watermark-sized text, as positioned, editable
//! shapes to avoid reflowing overlays; other pages flow with images inline.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod pptx;
mod zip;

pub use pptx::pptx;
pub use zip::Zip;

mod as_docx;
mod as_html;
mod as_markdown;
mod as_rtf;
#[path = "layout.rs"]
mod layout_impl;

mod layout {
    use super::layout_impl::{Item, Page, Run, body_size, paragraphs, tables};
    pub use super::layout_impl::{Block, Cell, Image, Page, Table};
    #[cfg(test)]
    pub(crate) use super::layout_impl::{LOOKUP_STEPS, MAX_COLS};
    pub(crate) use super::layout_impl::Run;
    pub(crate) use super::layout_impl::{Run as Run_inner, body_size, count_lookup_steps, paragraphs, tables};

    fn level(b: &super::layout_impl::Block, body: f64) -> u8 {
        let short = b.text.len() < 200;
        if short && b.size >= body * 1.6 {
            1
        } else if short && (b.size >= body * 1.25 || (b.bold && b.size >= body && b.text.len() < 80 && !b.text.ends_with('.'))) {
            2
        } else {
            0
        }
    }

    fn para_level(runs: &[Run], body: f64) -> u8 {
        let Some(first) = runs.first() else { return 0 };
        let marker = |r: &Run| r.block.size < first.block.size * 0.9 && r.block.text.trim().chars().count() <= 3;
        runs.iter().filter(|r| !marker(r)).map(|r| level(r.block, body)).min().unwrap_or(0)
    }

    pub(crate) fn items(pages: &[Page]) -> Vec<Item<'_>> {
        let body = body_size(pages);
        let mut out = Vec::new();
        for (i, p) in pages.iter().enumerate() {
            if i > 0 {
                out.push(Item::PageBreak);
            }
            let (tables, consumed) = tables(&p.blocks);
            for runs in paragraphs(&p.blocks, &consumed) {
                let lvl = para_level(&runs, body);
                out.push(Item::Para(runs, lvl));
            }
            out.extend(tables.into_iter().map(Item::Table));
            out.extend(p.images.iter().map(Item::Img));
        }
        out
    }
}

pub use as_docx::docx;
pub(crate) use as_docx::hex;
#[cfg(test)]
use as_docx::{MAX_MEDIA, docx_table};
pub(crate) use as_html::esc;
#[cfg(test)]
use as_html::{allowed, base64};
pub use as_html::{html, html_simple};
pub use as_markdown::markdown;
#[cfg(test)]
use as_markdown::{emphasis, paragraph};
pub use as_rtf::rtf;
pub use layout::{Block, Cell, Image, Page, Table};
#[cfg(test)]
use layout::{LOOKUP_STEPS, MAX_COLS, body_size};
pub(crate) use layout::{Run, paragraphs, tables};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod items_tests {
    use super::*;
    use layout::{Item, items};

    #[test]
    fn items_keep_reading_order_not_top_edge() {
        let paragraph = |text: &str, y: f64| Block {
            text: text.to_string(),
            rect: [72.0, y, 500.0, y + 11.0],
            size: 11.0,
            bold: false,
            italic: false,
            color: [0.0; 3],
            rotation: 0.0,
            space_around: [false; 2],
        };
        let page = Page {
            width: 612.0,
            height: 792.0,
            blocks: vec![paragraph("First half in reading order.", 500.0), paragraph("Second half in reading order.", 700.0)],
            images: Vec::new(),
            background: None,
        };
        let texts: Vec<String> = items(std::slice::from_ref(&page))
            .into_iter()
            .filter_map(|item| match item {
                Item::Para(runs, _) => Some(runs.iter().map(|run| run.text.clone()).collect::<Vec<_>>().join(" ")),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["First half in reading order.".to_string(), "Second half in reading order.".to_string()]);
    }
}
