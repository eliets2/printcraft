//! Feeds arbitrary bytes, read as a script, to the JavaScript run entry with fixed event and document values.
//!
//! The run's result is discarded: the target only checks that no input panics or hangs past the limits.
#![no_main]

use libfuzzer_sys::fuzz_target;
use pdfcraft_js::{DocInfo, Event, Limits, run};

/// The largest script the target runs (64 KiB); larger inputs are skipped.
const MAX_INPUT: usize = 64 << 10;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }
    let script = String::from_utf8_lossy(data).into_owned();
    let event = Event {
        name: "Keystroke".into(),
        kind: "Field".into(),
        target: Some("field1".into()),
        value: "value".into(),
        change: String::new(),
        will_commit: true,
    };
    let _outcome = run(&script, &event, &DocInfo::default(), &[], &[], Limits::default());
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_script_fuzz_target() {
        let _outcome = run(
            "",
            &Event {
                name: "Keystroke".into(),
                kind: "Field".into(),
                target: Some("field1".into()),
                value: "value".into(),
                change: String::new(),
                will_commit: true,
            },
            &DocInfo::default(),
            &[],
            &[],
            Limits::default(),
        );
        let source = include_str!("run_script.rs");
        let head = source.split("#[cfg(test)]").next().unwrap_or(source);
        assert!(
            head.contains("#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]"),
            "fuzz target crate root must carry the repository-required deny attributes"
        );
    }
}
