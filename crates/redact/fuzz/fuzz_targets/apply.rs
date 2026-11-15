//! Feeds arbitrary bytes, read as a PDF, to the redaction marks and the apply entry.
//!
//! Results are discarded: the target checks that no input panics.
#![no_main]

use std::sync::Arc;

use libfuzzer_sys::fuzz_target;
use pdfcraft_cos::Document;
use pdfcraft_redact::{apply, marks};

/// The largest input the target opens (16 MiB); larger inputs are skipped.
const MAX_INPUT: usize = 16 << 20;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }
    let Ok(mut doc) = Document::open(Arc::new(data.to_vec())) else {
        return;
    };
    let _count = marks(&doc).len();
    let _result = apply(&mut doc, None);
});

#[cfg(test)]
mod tests {
    #[test]
    fn apply_fuzz_target_has_clippy_deny_attributes() {
        let source = include_str!("apply.rs");
        assert!(
            source.contains(
                "#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, \
                 clippy::unimplemented, clippy::todo, clippy::unreachable)]"
            ),
            "fuzz crate root must carry the crate-level clippy deny attributes"
        );
    }
}
