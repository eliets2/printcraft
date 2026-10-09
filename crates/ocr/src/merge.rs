//! Combining engines. "ROVER" merges two engines' readings of one page: every primary word
//! adopts the best-overlapping unused secondary word when their boxes agree enough (IoU, below);
//! the higher confidence wins the text, box and confidence (a tie keeps the primary) and the
//! surviving word is named `"ROVER"`; secondary words nothing claimed are appended, keeping
//! their own engine's name.

use crate::{Line, OcrError, OcrImage, RecognizeOptions, Recognizer, Word};

/// How the chosen engines combine. The default is [`MergeStrategy::PrimaryOnly`] with the
/// default engine (ocrs), which is today's behavior.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MergeStrategy {
    /// Only the primary engine runs.
    #[default]
    PrimaryOnly,
    /// The secondary engine joins only when the primary's reading looks weak: its mean word
    /// confidence is under 70, or its confidence is unknown (words were read but none reports
    /// a confidence — the ocrs engine). An unknown reading is never counted as 100; an empty
    /// primary keeps the fast path (no words, nothing to double-check).
    ConfidenceWeighted,
    /// Both engines always run and their readings merge (with no secondary engine this
    /// degrades to primary only).
    RoverVote,
}

/// The mean of the confidences the words report, 0–100; `None` when the reading's confidence
/// is UNKNOWN — words were read but none reports a confidence (the ocrs engine's words).
/// Unknown is never counted as 100: there is no low-confidence signal, but a reading whose
/// confidence nobody knows is not therefore confident, so a confidence-weighted strategy
/// answers it by asking the secondary engine. An empty reading is `Some(100.0)`: no words
/// were read, so there is nothing to double-check and the fast path stands.
pub fn mean_confidence(lines: &[Line]) -> Option<f32> {
    let mut total = 0usize;
    let mut reported = (0.0f64, 0usize);
    for word in lines.iter().flat_map(|l| &l.words) {
        total += 1;
        if let Some(c) = word.confidence {
            reported.0 += c as f64;
            reported.1 += 1;
        }
    }
    match (total, reported.1) {
        (0, _) => Some(100.0),
        (_, 0) => None,
        (_, n) => Some((reported.0 / n as f64) as f32),
    }
}

/// Intersection over union of two `[left, top, right, bottom]` boxes. An empty intersection is
/// 0; degenerate or NaN boxes give 0 rather than panicking or poisoning the comparison.
pub fn iou(a: &[f32; 4], b: &[f32; 4]) -> f64 {
    let l = a[0].max(b[0]);
    let t = a[1].max(b[1]);
    let r = a[2].min(b[2]);
    let btm = a[3].min(b[3]);
    if !(r > l && btm > t) {
        return 0.0; // disjoint, touching, inverted, or NaN (all comparisons false)
    }
    let inter = (r - l) as f64 * (btm - t) as f64;
    let area = |x: &[f32; 4]| ((x[2] - x[0]).max(0.0) as f64) * ((x[3] - x[1]).max(0.0) as f64);
    let union = area(a) + area(b) - inter;
    if union <= 0.0 {
        return 0.0;
    }
    inter / union
}

/// The most primary×secondary word comparisons one ROVER merge will attempt. Known limit: a
/// merge over this cap is skipped — the primary reading stands alone — so two enormous
/// readings cannot turn the merge into quadratic work without end. A real page is a few
/// hundred words on each side, orders of magnitude under the cap.
pub const ROVER_MAX_COMPARISONS: usize = 100_000_000;

/// True when `a` beats `b` on confidence: a reported confidence beats none (a known reading
/// is preferred over an unknown one), and NaN confidences never win. Where neither side
/// reports a confidence the comparison is not meaningful and the primary word stands —
/// `false` for (None, None).
fn higher(a: Option<f32>, b: Option<f32>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.partial_cmp(&b) == Some(std::cmp::Ordering::Greater),
        (Some(_), None) => true,
        _ => false,
    }
}

/// Merge the two engines' readings. Primary words keep their order and lines; a word is
/// replaced by its overlapping secondary (IoU >= 0.5) only when that secondary is more
/// confident, and either way the merged word's [`Word::source`] is `"ROVER"`. Secondary words
/// no primary claimed are appended, in their own reading order and lines, under their own
/// engine's name (`secondary_id`). Known limit: over [`ROVER_MAX_COMPARISONS`] comparisons
/// (word counts multiplied) the primary reading is kept as is, unmerged.
pub fn rover_merge(primary: Vec<Line>, secondary: Vec<Line>, secondary_id: &str) -> Vec<Line> {
    let primary_words: usize = primary.iter().map(|l| l.words.len()).sum();
    let secondary_words: usize = secondary.iter().map(|l| l.words.len()).sum();
    if primary_words.saturating_mul(secondary_words) > ROVER_MAX_COMPARISONS {
        return primary; // over the comparison cap: the primary stands alone
    }
    let mut primary = primary;
    // The secondary's words, marked `None` once a primary word claimed them, grouped by their
    // own lines so the unclaimed ones can be appended in order.
    let mut unused: Vec<Vec<Option<Word>>> = secondary.into_iter().map(|l| l.words.into_iter().map(Some).collect()).collect();
    for line in &mut primary {
        for word in &mut line.words {
            // The best-overlapping unused secondary word, if any beats IoU 0.5.
            let mut best: Option<(f64, usize, usize)> = None;
            for (li, words) in unused.iter().enumerate() {
                for (wi, slot) in words.iter().enumerate() {
                    let Some(w) = slot else { continue };
                    let score = iou(&word.rect, &w.rect);
                    if score >= 0.5 && best.is_none_or(|(b, _, _)| score > b) {
                        best = Some((score, li, wi));
                    }
                }
            }
            let Some((_, li, wi)) = best else { continue };
            let claimed = unused.get_mut(li).and_then(|l| l.get_mut(wi)).and_then(|slot| slot.take());
            let Some(claim) = claimed else { continue };
            if higher(claim.confidence, word.confidence) {
                *word = Word { source: "ROVER".into(), ..claim };
            } else {
                // Tie (or the primary wins): keep the primary's text, box and confidence, but
                // record that this reading is the engines' agreed one.
                word.source = "ROVER".into();
            }
        }
    }
    // Append the secondary words nothing claimed, line by line, under their engine's name
    // (`secondary_id` backfills words that name no source of their own).
    let mut merged = primary;
    for line in unused {
        let words: Vec<Word> = line
            .into_iter()
            .flatten()
            .map(|mut w| {
                if w.source.is_empty() {
                    w.source = secondary_id.into();
                }
                w
            })
            .collect();
        if !words.is_empty() {
            merged.push(Line { words });
        }
    }
    merged
}

/// Run the engines per `strategy` and merge. Returns the lines and whether the secondary
/// engine actually ran. A secondary engine that is configured but fails is an honest
/// [`OcrError`], never a silently partial reading.
pub fn recognize_with_strategy(
    primary: &dyn Recognizer,
    secondary: Option<&dyn Recognizer>,
    strategy: MergeStrategy,
    image: &OcrImage,
    options: &RecognizeOptions,
) -> Result<(Vec<Line>, bool), OcrError> {
    match strategy {
        MergeStrategy::PrimaryOnly => Ok((primary.recognize(image, options)?, false)),
        MergeStrategy::RoverVote => match secondary {
            None => Ok((primary.recognize(image, options)?, false)),
            Some(sec) => {
                let p = primary.recognize(image, options)?;
                let s = sec.recognize(image, options)?;
                Ok((rover_merge(p, s, sec.id()), true))
            }
        },
        MergeStrategy::ConfidenceWeighted => {
            let p = primary.recognize(image, options)?;
            match mean_confidence(&p) {
                // Confident enough — or an empty reading: the primary stands alone.
                Some(mean) if mean >= 70.0 => Ok((p, false)),
                // A weak mean, or an unknown confidence (words, none reported): the secondary
                // runs.
                _ => match secondary {
                    None => Ok((p, false)),
                    Some(sec) => {
                        let s = sec.recognize(image, options)?;
                        Ok((rover_merge(p, s, sec.id()), true))
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A fake engine: returns canned lines, counts recognize calls, never available.
    struct Fake {
        id: &'static str,
        lines: Vec<Line>,
        calls: AtomicUsize,
        fail: bool,
    }

    impl Fake {
        fn new(id: &'static str, words: &[(&str, [f32; 4], f32)]) -> Fake {
            Fake {
                id,
                lines: vec![Line { words: words.iter().map(|(t, r, c)| Word::new(*t, *r, id).with_confidence(*c)).collect() }],
                calls: AtomicUsize::new(0),
                fail: false,
            }
        }

        fn empty(id: &'static str) -> Fake {
            Fake { id, lines: Vec::new(), calls: AtomicUsize::new(0), fail: false }
        }

        /// An engine like ocrs: words, but no confidence reported on any of them.
        fn unknown(id: &'static str, n: usize) -> Fake {
            Fake {
                id,
                lines: vec![Line {
                    words: (0..n).map(|i| Word::new(format!("w{i}"), [i as f32 * 10.0, 0.0, i as f32 * 10.0 + 8.0, 10.0], id)).collect(),
                }],
                calls: AtomicUsize::new(0),
                fail: false,
            }
        }
    }

    impl Recognizer for Fake {
        fn id(&self) -> &'static str {
            self.id
        }
        fn available(&self) -> bool {
            !self.fail
        }
        fn languages(&self) -> Vec<(String, String)> {
            vec![]
        }
        fn recognize(&self, _image: &OcrImage, _options: &RecognizeOptions) -> Result<Vec<Line>, OcrError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(OcrError::Recognize("the fake engine failed".into()));
            }
            Ok(self.lines.clone())
        }
    }

    fn image() -> OcrImage {
        OcrImage::new(4, 4, vec![255; 64]).unwrap()
    }

    fn words(lines: &[Line]) -> Vec<&Word> {
        lines.iter().flat_map(|l| &l.words).collect()
    }

    #[test]
    fn iou_of_disjoint_touching_and_nan_boxes_is_zero() {
        let a = [0.0, 0.0, 10.0, 10.0];
        assert_eq!(iou(&a, &[20.0, 0.0, 30.0, 10.0]), 0.0, "disjoint");
        assert_eq!(iou(&a, &[10.0, 0.0, 20.0, 10.0]), 0.0, "touching edges are an empty intersection");
        assert_eq!(iou(&a, &[f32::NAN, 0.0, 10.0, 10.0]), 0.0, "NaN boxes score 0, not a panic");
        assert_eq!(iou(&a, &a), 1.0, "identical boxes");
        assert!((iou(&a, &[5.0, 0.0, 15.0, 10.0]) - 1.0 / 3.0).abs() < 1e-9, "half overlap is IoU 1/3");
    }

    /// With nothing on the primary side, every secondary word is appended under the secondary
    /// engine's own name.
    #[test]
    fn rover_appends_everything_when_the_primary_is_empty() {
        let primary = Vec::new();
        let secondary = vec![Line { words: vec![Word::new("sole", [0.0, 0.0, 9.0, 9.0], "tesseract").with_confidence(80.0)] }];
        let merged = rover_merge(primary, secondary, "tesseract");
        assert_eq!(words(&merged).len(), 1);
        assert_eq!(merged[0].words[0].text, "sole");
        assert_eq!(merged[0].words[0].source, "tesseract", "unclaimed words keep their own engine");
    }

    /// Non-overlapping secondary words never displace primary words; they are appended.
    #[test]
    fn rover_appends_non_overlapping_secondary_words() {
        let primary = vec![Line { words: vec![Word::new("kept", [0.0, 0.0, 10.0, 10.0], "ocrs").with_confidence(50.0)] }];
        let secondary = vec![Line { words: vec![Word::new("extra", [100.0, 100.0, 120.0, 110.0], "tesseract").with_confidence(99.0)] }];
        let merged = rover_merge(primary, secondary, "tesseract");
        let w = words(&merged);
        assert_eq!(w.len(), 2);
        assert_eq!((w[0].text.as_str(), w[0].source.as_str()), ("kept", "ocrs"), "the more confident but disjoint word does not win");
        assert_eq!((w[1].text.as_str(), w[1].source.as_str()), ("extra", "tesseract"));
    }

    /// The higher-confidence word of an overlapping pair wins text, box and confidence, and is
    /// named ROVER.
    #[test]
    fn rover_higher_confidence_wins_and_is_named_rover() {
        let primary = vec![Line { words: vec![Word::new("helo", [0.0, 0.0, 10.0, 10.0], "ocrs").with_confidence(60.0)] }];
        let secondary = vec![Line { words: vec![Word::new("hello", [1.0, 0.0, 11.0, 10.0], "tesseract").with_confidence(95.0)] }];
        let merged = rover_merge(primary, secondary, "tesseract");
        let w = words(&merged);
        assert_eq!(w.len(), 1, "the secondary word was claimed, not appended");
        assert_eq!(w[0].text, "hello");
        assert_eq!(w[0].rect, [1.0, 0.0, 11.0, 10.0]);
        assert_eq!(w[0].confidence, Some(95.0));
        assert_eq!(w[0].source, "ROVER");
    }

    /// A tie keeps the primary word (still named ROVER, as an agreed reading).
    #[test]
    fn rover_tie_keeps_the_primary() {
        let primary = vec![Line { words: vec![Word::new("primary", [0.0, 0.0, 10.0, 10.0], "ocrs").with_confidence(80.0)] }];
        let secondary = vec![Line { words: vec![Word::new("secondary", [0.0, 0.0, 10.0, 10.0], "tesseract").with_confidence(80.0)] }];
        let merged = rover_merge(primary, secondary, "tesseract");
        let w = words(&merged);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].text, "primary");
        assert_eq!(w[0].source, "ROVER");
    }

    /// Boxes under the 0.5 IoU bar never merge, even when the secondary is more confident.
    #[test]
    fn rover_below_half_iou_does_not_merge() {
        let primary = vec![Line { words: vec![Word::new("kept", [0.0, 0.0, 10.0, 10.0], "ocrs").with_confidence(50.0)] }];
        // IoU 10·10 / (100 + 169 - 25) = 100/244 ≈ 0.41 < 0.5.
        let secondary = vec![Line { words: vec![Word::new("other", [5.0, 0.0, 23.0, 20.0], "tesseract").with_confidence(99.0)] }];
        let merged = rover_merge(primary, secondary, "tesseract");
        let w = words(&merged);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].text, "kept");
        assert_eq!(w[0].source, "ocrs");
    }

    /// Merged words keep their line; the merge never panics on NaN boxes.
    #[test]
    fn rover_survives_nan_boxes() {
        let primary = vec![Line { words: vec![Word::new("nan", [f32::NAN, 0.0, 10.0, 10.0], "ocrs").with_confidence(50.0)] }];
        let secondary = vec![Line { words: vec![Word::new("real", [0.0, 0.0, 10.0, 10.0], "tesseract").with_confidence(90.0)] }];
        let merged = rover_merge(primary, secondary, "tesseract");
        let w = words(&merged);
        assert_eq!(w.len(), 2, "a NaN primary box merges with nothing");
        assert_eq!(w[0].text, "nan");
        assert_eq!(w[1].text, "real");
    }

    /// Only reported confidences count. An empty reading is Some(100) (nothing to
    /// double-check); words without a single confidence are UNKNOWN (`None`), never 100.
    #[test]
    fn mean_confidence_distinguishes_empty_from_unknown() {
        assert_eq!(mean_confidence(&[]), Some(100.0));
        assert_eq!(mean_confidence(&[Line { words: vec![Word::new("a", [0.0; 4], "ocrs")] }]), None, "words, no confidences: unknown");
        let lines = vec![Line {
            words: vec![
                Word::new("a", [0.0; 4], "e").with_confidence(40.0),
                Word::new("b", [0.0; 4], "e").with_confidence(80.0),
                Word::new("c", [0.0; 4], "e"),
            ],
        }];
        assert_eq!(mean_confidence(&lines), Some(60.0), "reported confidences are averaged");
    }

    /// ROVER's preference needs a meaningful comparison: a known confidence beats an unknown
    /// one (the ocrs primary yields to a confident tesseract reading), and None vs None keeps
    /// the primary.
    #[test]
    fn rover_known_confidence_beats_unknown_and_none_vs_none_keeps_the_primary() {
        let primary = vec![Line { words: vec![Word::new("helo", [0.0, 0.0, 10.0, 10.0], "ocrs")] }];
        let secondary = vec![Line { words: vec![Word::new("hello", [0.0, 0.0, 10.0, 10.0], "tesseract").with_confidence(95.0)] }];
        let merged = rover_merge(primary, secondary, "tesseract");
        assert_eq!(
            (merged[0].words[0].text.as_str(), merged[0].words[0].confidence, merged[0].words[0].source.as_str()),
            ("hello", Some(95.0), "ROVER")
        );
        let primary = vec![Line { words: vec![Word::new("primary", [0.0, 0.0, 10.0, 10.0], "ocrs")] }];
        let secondary = vec![Line { words: vec![Word::new("secondary", [0.0, 0.0, 10.0, 10.0], "tesseract")] }];
        let merged = rover_merge(primary, secondary, "tesseract");
        assert_eq!(
            (merged[0].words[0].text.as_str(), merged[0].words[0].source.as_str()),
            ("primary", "ROVER"),
            "unknown vs unknown is not a comparison: the primary stands"
        );
    }

    /// Over the comparison cap the merge is skipped and the primary reading stands alone (a
    /// documented known limit, bounding the merge's quadratic core).
    #[test]
    fn rover_over_the_comparison_cap_keeps_the_primary() {
        // 11 000 × 11 000 = 121 M comparisons > the 100 M cap.
        let words = |id: &str| (0..11_000).map(|i| Word::new(format!("w{i}"), [i as f32, 0.0, i as f32 + 5.0, 10.0], id)).collect::<Vec<_>>();
        let primary = vec![Line { words: words("ocrs") }];
        let secondary = vec![Line { words: words("tesseract") }];
        let merged = rover_merge(primary, secondary, "tesseract");
        assert_eq!(merged.len(), 1);
        assert!(merged[0].words.iter().all(|w| w.source == "ocrs"), "every primary word stands, unmerged");
        assert_eq!(merged[0].words.len(), 11_000);
        // Under the cap, merging still happens (a small overlap is replaced and named ROVER).
        let small_primary = vec![Line { words: vec![Word::new("kept", [0.0, 0.0, 10.0, 10.0], "ocrs").with_confidence(50.0)] }];
        let small_secondary = vec![Line { words: vec![Word::new("wins", [0.0, 0.0, 10.0, 10.0], "tesseract").with_confidence(95.0)] }];
        let merged = rover_merge(small_primary, small_secondary, "tesseract");
        assert_eq!((merged[0].words[0].text.as_str(), merged[0].words[0].source.as_str()), ("wins", "ROVER"));
    }

    #[test]
    fn primary_only_never_runs_the_secondary() {
        let primary = Fake::new("ocrs", &[("w", [0.0; 4], 90.0)]);
        let secondary = Fake::empty("tesseract");
        let (lines, used) =
            recognize_with_strategy(&primary, Some(&secondary), MergeStrategy::PrimaryOnly, &image(), &RecognizeOptions::default()).unwrap();
        assert_eq!(primary.calls.load(Ordering::SeqCst), 1);
        assert_eq!(secondary.calls.load(Ordering::SeqCst), 0);
        assert!(!used);
        assert_eq!(words(&lines).len(), 1);
    }

    #[test]
    fn confidence_weighted_runs_the_secondary_only_on_a_weak_primary() {
        let weak = Fake::new("ocrs", &[("w", [0.0; 4], 40.0)]);
        let strong = Fake::new("ocrs", &[("w", [0.0; 4], 95.0)]);
        let secondary = Fake::empty("tesseract");
        let (_, used) =
            recognize_with_strategy(&weak, Some(&secondary), MergeStrategy::ConfidenceWeighted, &image(), &RecognizeOptions::default()).unwrap();
        assert!(used, "mean 40 < 70 → the secondary joins");
        let (_, used) =
            recognize_with_strategy(&strong, Some(&secondary), MergeStrategy::ConfidenceWeighted, &image(), &RecognizeOptions::default()).unwrap();
        assert!(!used, "mean 95 >= 70 → primary alone");
        // An empty primary counts as 100: no signal, no secondary run.
        let empty = Fake::empty("ocrs");
        let (_, used) =
            recognize_with_strategy(&empty, Some(&secondary), MergeStrategy::ConfidenceWeighted, &image(), &RecognizeOptions::default()).unwrap();
        assert!(!used);
        assert_eq!(secondary.calls.load(Ordering::SeqCst), 1, "only the weak page ran the secondary");
    }

    /// The ocrs engine reports no confidences: unknown is not 100, so the secondary runs —
    /// and its known reading wins the overlapping word (the ensemble's whole point).
    #[test]
    fn confidence_weighted_runs_the_secondary_on_an_unknown_primary() {
        let ocrs_like = Fake::unknown("ocrs", 1);
        let secondary = Fake::new("tesseract", &[("hello", [0.0, 0.0, 10.0, 10.0], 95.0)]);
        let (lines, used) =
            recognize_with_strategy(&ocrs_like, Some(&secondary), MergeStrategy::ConfidenceWeighted, &image(), &RecognizeOptions::default()).unwrap();
        assert!(used, "an unknown confidence asks the secondary");
        assert_eq!((lines[0].words[0].text.as_str(), lines[0].words[0].confidence), ("hello", Some(95.0)));
        assert_eq!(lines[0].words[0].source, "ROVER");
        assert_eq!(secondary.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn rover_vote_always_runs_both_and_degrades_without_a_secondary() {
        let primary = Fake::new("ocrs", &[("w", [0.0; 4], 95.0)]);
        let secondary = Fake::empty("tesseract");
        let (_, used) =
            recognize_with_strategy(&primary, Some(&secondary), MergeStrategy::RoverVote, &image(), &RecognizeOptions::default()).unwrap();
        assert!(used);
        assert_eq!(secondary.calls.load(Ordering::SeqCst), 1);
        // No secondary: primary only.
        let (_, used) = recognize_with_strategy(&primary, None, MergeStrategy::RoverVote, &image(), &RecognizeOptions::default()).unwrap();
        assert!(!used);
    }

    /// A configured secondary that fails is a typed failure, not a silent partial reading.
    #[test]
    fn a_failing_secondary_is_an_error_not_a_silent_degrade() {
        let primary = Fake::new("ocrs", &[("w", [0.0; 4], 95.0)]);
        let mut secondary = Fake::empty("tesseract");
        secondary.fail = true;
        let r = recognize_with_strategy(&primary, Some(&secondary), MergeStrategy::RoverVote, &image(), &RecognizeOptions::default());
        assert!(r.is_err());
    }
}
