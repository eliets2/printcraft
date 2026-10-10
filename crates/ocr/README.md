# pdfcraft-ocr

Layer L4 (may use `fonts` for the text layers' metrics): Scan & OCR — recognising the words in
a page raster and turning them back into page content (execution plan M10).

```rust
// The caller renders a page to pixels and hands them over:
let image = OcrImage::new(w, h, rgba)?;                  // validated by construction
let lines = recognizer.recognize(&image, &options)?;     // Vec<Line>: words in reading order
let placed: Vec<PlacedWord> = …;                         // PlacedWord::place maps boxes to user space
let layer = text_layer(&placed);                         // invisible text: a searchable image
let layer = visible_text_layer(&placed);                 // visible text: the editable output
```

- **Engines behind one trait.** A [`Recognizer`] reports its `id`, whether it is `available`,
  the languages it reads, and reads `Vec<Line>` from an image. Two exist: the pure-Rust
  `OcrsRecognizer` (the `ocrs` crate, MIT/Apache-2.0, with its pre-trained models,
  CC-BY-SA-4.0, fetched by `cargo xtask models`; found via `$PDFCRAFT_MODELS` or `models/`
  beside the executable — see ATTRIBUTION.toml), and — off the web build —
  `tesseract::TesseractCli`, which drives an installed `tesseract` binary as an external
  process: never linked, bundled or downloaded. Implementations are shared across threads
  (`Send + Sync`); nothing spawns on wasm32.
- **The tesseract process is fenced in.** The program comes from a user-configured location or
  the PATH, never from document data (the engine validates a configured path as an absolute
  path to an existing executable file before it is used). The language code is allow-listed
  (`tesseract::language_code`, 12 codes) before anything reaches a process; the page image
  goes to a private temp file (a fresh 0700 directory holding a 0600 file on Unix) that is
  deleted afterwards; the run has a 120 s timeout, a 64 MiB output cap and a 50 000-word
  reading cap; every failure comes back through `OcrError` with its cause named. Availability
  is probed with `--list-langs`; a failed probe is remembered for 30 s, so an install that
  appears later is found without a restart.
- **Confidence drives review.** One classifier (`band_for`) puts a word in the High
  (≥ 90), Medium (70–89) or Low (< 70) band — the review overlay's colours, the legend and the
  counts; the batch flag rule (`low_confidence`, default threshold 60) is deliberately a
  separate threshold. The `ocrs` models report no confidence (its words carry `None`);
  a reading whose confidence is unknown is treated as unknown, never as good.
- **Two engines can be merged (ROVER).** Strategies: `PrimaryOnly` (the default), and two
  ensembles — `ConfidenceWeighted` (the secondary engine joins when the primary's mean
  confidence is under 70 or unknown; an empty primary keeps the fast path) and `RoverVote`
  (both engines always run). The merge lets every primary word adopt the best-overlapping
  unused secondary word at IoU ≥ 0.5; the higher confidence wins text, box and confidence (a
  tie keeps the primary), the survivor is named `"ROVER"`, and unclaimed secondary words are
  appended under their engine's name. A merge is capped at 100 M word-pair comparisons (over
  it, the primary reading stands alone), and a failing partner degrades to the primary with a
  recorded note — a page is never skipped because a partner could not read it.
- **Preprocessing between the raster and recognition**, a pure function in fixed order: DPI
  normalization to 300 (always; a scan at or above it is left alone), then the opt-in steps —
  orientation (90°/180°/270° from projection-profile banding), deskew (± 5°, shear-scored,
  below 0.1° is a no-op), denoise (3×3 median) and binarization (Sauvola, window 8, k 0.34).
  Every geometric step composes one exact affine `InverseTransform`, and callers map every
  word box back to the original raster before placing it. The heuristics are conservative and
  panic-free: too small to judge, blank, or no clear winner is a no-op, never a guess.
- **Text layers in standard Helvetica** (`/PCHelv`, WinAnsiEncoding, marked-content `/OCR` so
  the OCR text can be told apart): `text_layer` writes invisible text (rendering mode 3), each
  word's em square stretched to its box, so selection and search highlight the right place;
  `visible_text_layer` writes visible text whose size comes from the box height and whose
  horizontal scaling is an explicit `Tz` percentage computed from real Helvetica metrics.
  Words with non-finite or empty geometry are skipped, and rotated pages keep their reading
  direction (each word carries `across`/`up` vectors, not just an origin).
- **Hostile input is rejected, not panicked at.** `OcrImage` holds private, validated state:
  non-zero sides, a 10 000-pixel side cap, a 64-megapixel area cap (checked arithmetic — the
  counts come from documents, so they are untrusted), and an exact buffer length.

## Where it is wired

The engine's `ocr` module renders pages into `OcrImage`s, runs jobs (`OcrJob::run`) and applies
the words: searchable-image mode draws the invisible layer over the page's own image and leaves
that image untouched; editable mode writes a NEW `<name>_ocr.pdf` of visible text (the scan
image is not carried over, the source is never overwritten). Skip probes keep pages — and
whole files — that already contain text out of the work unless Force OCR overrides them. The
UI reviews the words before applying them (the OCR Verify screen: confidence-coloured boxes,
corrections, a per-language user dictionary, Re-OCR of a region or a page, Accept/Reject), a
multiple-file run writes searchable copies as it goes, and the automation tools `ocr_recognize`,
`ocr_words`, `ocr_status` and `ocr_recognize_files` drive the same paths headlessly.

Not yet: languages beyond the tested set (the `ocrs` models read only unaccented Latin; the
Tesseract side is bounded by its allow-list and whatever its installed language packs cover),
OCR in the browser (the tesseract module is compiled out for wasm32), the deep scan-cleanup
options (descreen, background removal, edge-shadow removal), and text layers in faces other
than standard Helvetica.
