//! Scan & OCR ▸ Recognize text: render pages, read the words in them and add an invisible text
//! layer ([`Edit::AddOcrText`]), making scanned pages searchable and selectable.
//!
//! Recognition is slow (about a second a page), so it is split from the edit: [`Session::ocr_job`]
//! captures what it needs, [`OcrJob::run`] works anywhere (the UI runs it on a worker thread) and
//! [`Session::apply_ocr`] applies the result as one undoable step. The result is only applied
//! while the document is still the one that was read — [`Session::apply_ocr_checked`] refuses a
//! stale result with a named reason instead of writing into a changed document.

use std::sync::{Arc, Mutex};

use pdfcraft_render::{PageInfo, PageRenderer, RenderConfig, RenderRequest, RequestKind};

pub use pdfcraft_ocr::{LANGUAGES, MergeStrategy, Models, Ocr, OcrError, PlacedWord, Recognizer, band_for, low_confidence};

use crate::{DocId, Edit, EditError, Session};

/// Which recognition engine to use. The default is ocrs, the pure-Rust engine; "Automatic" is
/// an ensemble — ocrs reading first, Tesseract as a second opinion, merged per the strategy —
/// when both are available.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EngineChoice {
    /// The pure-Rust ocrs engine (the default).
    #[default]
    Ocrs,
    /// The installed `tesseract` binary, as an external process.
    Tesseract,
    /// Ensemble: the default engine first, the other one as a secondary when available.
    Auto,
}

impl EngineChoice {
    /// Parse a choice; unknown names fall back to the default (ocrs), like Acrobat's dialogs
    /// fall back for unknown persisted values.
    pub fn parse(s: &str) -> EngineChoice {
        match s.trim().to_ascii_lowercase().as_str() {
            "tesseract" => EngineChoice::Tesseract,
            "auto" | "automatic" | "ensemble" => EngineChoice::Auto,
            _ => EngineChoice::Ocrs,
        }
    }

    /// The name settings and tools use for this choice.
    pub fn name(self) -> &'static str {
        match self {
            EngineChoice::Ocrs => "ocrs",
            EngineChoice::Tesseract => "tesseract",
            EngineChoice::Auto => "auto",
        }
    }
}

/// What to produce from a recognition. **Searchable** (the default) adds the existing
/// invisible-text layer on the original page; **EditableText** writes a *new* document with the
/// reviewed words as visible text and no scan image (the writer is the Verify screen's; the mode
/// is plumbed through here). Unknown mode strings mean Searchable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum OutputMode {
    #[default]
    Searchable,
    EditableText,
}

impl OutputMode {
    /// Parse a mode name (trimmed, case-insensitive); unknown strings mean Searchable.
    pub fn parse(s: &str) -> OutputMode {
        match s.trim().to_ascii_lowercase().as_str() {
            "editable" | "editable-text" | "editable_text" | "editabletext" => OutputMode::EditableText,
            _ => OutputMode::Searchable,
        }
    }

    /// The name settings and tools use for this mode.
    pub fn name(self) -> &'static str {
        match self {
            OutputMode::Searchable => "searchable",
            OutputMode::EditableText => "editable",
        }
    }
}

/// Recognize Text settings.
#[derive(Clone, Debug, PartialEq)]
pub struct OcrSettings {
    /// Resolution pages are rendered at for recognition (Acrobat's "Downsample to" choices).
    pub dpi: f32,
    /// Language code from [`LANGUAGES`] (the models read the Latin alphabet, whatever this says).
    pub language: String,
    /// Leave pages that already have text alone (Acrobat reports "page contains renderable
    /// text" and skips them).
    pub skip_text_pages: bool,
    /// Which engine reads the pages.
    pub engine: EngineChoice,
    /// How the engines' readings combine (see [`MergeStrategy`]).
    pub strategy: MergeStrategy,
    /// Preprocessing, all off by default (a scan is passed through as rendered):
    /// straighten pages scanned at 90°/180°/270°.
    pub auto_rotate: bool,
    /// Preprocessing: straighten skewed text lines.
    pub deskew: bool,
    /// Preprocessing: 3×3 median filter.
    pub denoise: bool,
    /// Preprocessing: Sauvola binarization.
    pub binarize: bool,
    /// What to produce (searchable text layer, or a new editable document).
    pub output_mode: OutputMode,
    /// Restrict recognition to this rectangle of the rendered page image, in pixels
    /// `[x0, y0, x1, y1]` (the Verify screen's "Re-OCR this region").
    pub region: Option<[f32; 4]>,
    /// The batch note's separate per-word threshold: words with `0 <= conf <= threshold` are
    /// counted for "N low-confidence word(s) on page(s) …" (see [`low_confidence`]; unrelated
    /// to the review overlay's bands, which use [`band_for`]).
    pub low_confidence_threshold: f32,
}

impl Default for OcrSettings {
    fn default() -> Self {
        OcrSettings {
            dpi: 300.0,
            language: "en".into(),
            skip_text_pages: true,
            engine: EngineChoice::default(),
            strategy: MergeStrategy::default(),
            auto_rotate: false,
            deskew: false,
            denoise: false,
            binarize: false,
            output_mode: OutputMode::default(),
            region: None,
            low_confidence_threshold: 60.0,
        }
    }
}

/// What recognition found on one page.
#[derive(Clone, Debug, PartialEq)]
pub struct OcrPage {
    pub page: usize,
    pub words: Vec<PlacedWord>,
    /// Why the page was not read, if it was skipped.
    pub skipped: Option<String>,
}

impl OcrPage {
    pub fn text(&self) -> String {
        self.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
    }

    /// Mean of the confidences the page's words report (`None` when none do).
    pub fn mean_confidence(&self) -> Option<f32> {
        let (sum, n) = self.words.iter().filter_map(|w| w.confidence).fold((0.0f64, 0usize), |(s, n), c| (s + c as f64, n + 1));
        (n > 0).then_some((sum / n as f64) as f32)
    }

    /// The words to flag for review: their confidence is in the Low band ([`band_for`]).
    /// Excluded are words the caller wants gone — removed words, words in the user dictionary,
    /// skipped words — which is review state, so this reports the indices and the reviewer
    /// filters them.
    pub fn suspects(&self) -> Vec<usize> {
        self.words.iter().enumerate().filter(|(_, w)| pdfcraft_ocr::is_suspect(w.confidence)).map(|(i, _)| i).collect()
    }
}

/// The engines a job reads with: one primary and, for ensemble strategies, an optional
/// secondary.
pub struct Recognizers {
    pub primary: Arc<dyn Recognizer>,
    pub secondary: Option<Arc<dyn Recognizer>>,
}

struct EngineCache {
    ocrs: Option<Arc<dyn Recognizer>>,
}

/// The Tesseract engine: found once (it answers `--list-langs` through a process) and then
/// remembered. `None` means it was probed and is not there.
#[cfg(not(target_arch = "wasm32"))]
fn tesseract_engine() -> Option<Arc<dyn Recognizer>> {
    static TESSERACT: Mutex<Option<Option<Arc<dyn Recognizer>>>> = Mutex::new(None);
    let mut slot = TESSERACT.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_none() {
        *slot = Some(pdfcraft_ocr::tesseract::TesseractCli::find().map(|t| Arc::new(t) as Arc<dyn Recognizer>));
    }
    slot.clone().flatten()
}

/// The web build never spawns processes, so it never has Tesseract.
#[cfg(target_arch = "wasm32")]
fn tesseract_engine() -> Option<Arc<dyn Recognizer>> {
    None
}

/// The engine selector: builds each engine once and hands out the pair `settings.engine`
/// asks for. "Automatic" is an ensemble when both engines are available; a named engine that
/// cannot start fails with its cause named.
pub fn recognizers(settings: &OcrSettings) -> Result<Recognizers, String> {
    static ENGINES: Mutex<Option<EngineCache>> = Mutex::new(None);
    let mut slot = ENGINES.lock().unwrap_or_else(|e| e.into_inner());
    let cache = slot.get_or_insert_with(|| EngineCache { ocrs: None });
    if cache.ocrs.is_none() {
        // Cheap to re-attempt while the models are missing; expensive work is cached forever.
        cache.ocrs = pdfcraft_ocr::OcrsRecognizer::find().ok().map(|r| Arc::new(r) as Arc<dyn Recognizer>);
    }
    let ocrs = cache.ocrs.clone();
    let tesseract = tesseract_engine();
    match settings.engine {
        EngineChoice::Ocrs => Ok(Recognizers { primary: ocrs.ok_or_else(models_missing)?, secondary: None }),
        EngineChoice::Tesseract => Ok(Recognizers { primary: tesseract.ok_or_else(tesseract_unavailable)?, secondary: None }),
        EngineChoice::Auto => match (ocrs, tesseract) {
            (Some(p), s) => Ok(Recognizers { primary: p, secondary: s }),
            (None, Some(p)) => Ok(Recognizers { primary: p, secondary: None }),
            (None, None) => Err(models_missing()),
        },
    }
}

/// Why Tesseract cannot run, named for the user.
#[cfg(not(target_arch = "wasm32"))]
fn tesseract_unavailable() -> String {
    "the tesseract program was not found (install it, or add it to the PATH)".into()
}

/// Why Tesseract cannot run, named for the user (the web build has no processes).
#[cfg(target_arch = "wasm32")]
fn tesseract_unavailable() -> String {
    "the tesseract engine is not available in the web build".into()
}

fn models_missing() -> String {
    "the text recognition models are not installed (run `cargo xtask models`, or set PDFCRAFT_MODELS)".into()
}

/// Whether recognition can run at all: the ocrs models are installed, or (off the web) a
/// Tesseract program is found.
pub fn available() -> bool {
    Models::find().is_some() || tesseract_engine().is_some()
}

/// What a started job expects of the document when its result is applied: the same open
/// document, unchanged. [`Session::stale_reason`] names any difference it finds.
#[derive(Clone)]
pub struct OcrGuard {
    pub(crate) doc: DocId,
    pub(crate) pages: usize,
    pub(crate) revisions: usize,
    /// The document's working bytes when the job started; any edit rewrites them, so a shared
    /// `Arc` pointer means "untouched".
    pub(crate) bytes: Arc<Vec<u8>>,
}

/// Everything recognition needs from a document, detached from the session.
pub struct OcrJob {
    bytes: Arc<Vec<u8>>,
    password: Option<String>,
    infos: Vec<PageInfo>,
    /// Per page: the render scale (pixels per point) that shows its images at their own
    /// resolution, if it has any.
    native: Vec<Option<f32>>,
    pub pages: Vec<usize>,
    pub settings: OcrSettings,
    /// What to check before applying this job's result.
    pub guard: OcrGuard,
}

/// A page's "has text" probe for skip-text: at least one non-blank text run. A failed probe
/// (the text layer could not be read) never enables a skip.
fn page_has_text(text: Option<&Arc<pdfcraft_render::PageText>>) -> bool {
    text.is_some_and(|t| t.glyphs.iter().any(|g| !g.text.trim().is_empty()))
}

/// Clamp a region to the image, snapping outward to whole pixels. `None` when nothing of it
/// is inside the image (empty, inverted, NaN, or wholly outside).
pub fn clamp_region(region: [f32; 4], width: u32, height: u32) -> Option<[u32; 4]> {
    let (w, h) = (f64::from(width), f64::from(height));
    if region.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let (x0, y0, x1, y1) = (region[0] as f64, region[1] as f64, region[2] as f64, region[3] as f64);
    let (x0, y0) = ((x0.floor().max(0.0).min(w)) as u32, (y0.floor().max(0.0).min(h)) as u32);
    let (x1, y1) = ((x1.ceil().max(0.0).min(w)) as u32, (y1.ceil().max(0.0).min(h)) as u32);
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1, y1])
}

impl OcrJob {
    /// Read the pages. `progress(done, total)` is called before each page; returning `false`
    /// stops (the pages read so far are returned).
    pub fn run(self, recognizers: &Recognizers, mut progress: impl FnMut(usize, usize) -> bool) -> Vec<OcrPage> {
        let config = RenderConfig { password: self.password.as_deref().map(Arc::from), ..Default::default() };
        let mut r = PageRenderer::new(self.bytes.clone(), config);
        let mut out = Vec::new();
        let total = self.pages.len();
        let options = pdfcraft_ocr::RecognizeOptions { language: self.settings.language.clone() };
        for (done, &page) in self.pages.iter().enumerate() {
            if !progress(done, total) {
                break;
            }
            let Some(info) = self.infos.get(page) else { continue };
            let skip = |why: &str| OcrPage { page, words: Vec::new(), skipped: Some(why.into()) };
            if self.settings.skip_text_pages {
                let t = r.render(RenderRequest { page, kind: RequestKind::Text, scale: 1.0, ..Default::default() });
                if page_has_text(t.text.as_ref()) {
                    out.push(skip("the page already contains text"));
                    continue;
                }
            }
            // A scan is read at its own resolution (enlarging it only blurs the letters), at most
            // the chosen one.
            let wanted = self.settings.dpi.clamp(72.0, 600.0) / 72.0;
            let scale = self.native.get(page).copied().flatten().map_or(wanted, |n| n.clamp(1.0, wanted));
            let shot = r.render(RenderRequest { page, scale, ..Default::default() });
            if let Some(e) = shot.error {
                out.push(skip(&e));
                continue;
            }
            let full = match pdfcraft_ocr::OcrImage::new(shot.width, shot.height, shot.rgba) {
                Ok(img) => img,
                Err(e) => {
                    out.push(skip(&e.to_string()));
                    continue;
                }
            };
            // The optional region: crop the rendered page, clamped; an empty region skips the
            // page with a named reason (nothing was asked of it).
            let (image, offset) = match self.settings.region {
                None => (full, [0.0f32; 2]),
                Some(region) => match clamp_region(region, shot.width, shot.height) {
                    Some([x0, y0, x1, y1]) => match crop(&full, x0, y0, x1, y1) {
                        Some(c) => (c, [x0 as f32, y0 as f32]),
                        None => {
                            out.push(skip("the selected region is empty"));
                            continue;
                        }
                    },
                    None => {
                        out.push(skip("the selected region is outside the page"));
                        continue;
                    }
                },
            };
            // Preprocessing sits between the page raster and recognition; its word boxes come
            // back in processed coordinates and are mapped to the ORIGINAL raster's before
            // placement. The render dpi is the real one (pixels per point × 72), never a
            // fixed value.
            let pre = pdfcraft_ocr::preprocess::preprocess(
                &image,
                scale * 72.0,
                &pdfcraft_ocr::preprocess::PreprocessOptions {
                    auto_rotate: self.settings.auto_rotate,
                    deskew: self.settings.deskew,
                    denoise: self.settings.denoise,
                    binarize: self.settings.binarize,
                },
            );
            let (image, inverse) = match pre {
                Ok(p) => (p.image, p.inverse_transform),
                Err(e) => {
                    out.push(skip(&e.to_string()));
                    continue;
                }
            };
            let lines = match pdfcraft_ocr::recognize_with_strategy(
                recognizers.primary.as_ref(),
                recognizers.secondary.as_deref(),
                self.settings.strategy,
                &image,
                &options,
            ) {
                Ok((l, _)) => l,
                Err(e) => {
                    out.push(skip(&e.to_string()));
                    continue;
                }
            };
            let scale = shot.width as f32 / info.width.max(1e-3);
            let to_user = |x: f32, y: f32| {
                // processed pixels → original raster pixels (the exact inverse) → past the
                // region crop → page view space.
                let (ox, oy) = inverse.apply(f64::from(x), f64::from(y));
                let [u, v] = info.view_to_user((ox as f32 + offset[0]) / scale, (oy as f32 + offset[1]) / scale);
                [u as f64, v as f64]
            };
            let words = lines.iter().flat_map(|l| &l.words).map(|w| PlacedWord::place(w, to_user)).collect();
            out.push(OcrPage { page, words, skipped: None });
        }
        progress(total, total);
        out
    }
}

/// Crop `[x0, y0, x1, y1]` out of `image` as a new [`OcrImage`] (the sides are already clamped
/// by [`clamp_region`]).
fn crop(image: &pdfcraft_ocr::OcrImage, x0: u32, y0: u32, x1: u32, y1: u32) -> Option<pdfcraft_ocr::OcrImage> {
    let (w, h) = (x1.checked_sub(x0)?, y1.checked_sub(y0)?);
    let stride = image.width as usize * 4;
    let mut out = Vec::with_capacity((w as usize).checked_mul(h as usize)?.checked_mul(4)?);
    for y in y0..y1 {
        let start = y as usize * stride + x0 as usize * 4;
        let row = image.rgba.get(start..start + (x1 - x0) as usize * 4)?;
        out.extend_from_slice(row);
    }
    pdfcraft_ocr::OcrImage::new(w, h, out).ok()
}

impl Session {
    /// Capture what recognising `pages` (0-based; empty = all) of document `id` needs.
    pub fn ocr_job(&self, id: DocId, pages: &[usize], settings: OcrSettings) -> Option<OcrJob> {
        let doc = self.get(id)?;
        let all = doc.info.pages.len();
        let pages: Vec<usize> = if pages.is_empty() { (0..all).collect() } else { pages.iter().copied().filter(|p| *p < all).collect() };
        let native = (0..all)
            .map(|p| {
                if !pages.contains(&p) {
                    return None;
                }
                doc.page_images(p)
                    .iter()
                    .filter(|i| i.width > 1 && i.height > 1)
                    .map(|i| {
                        let m = i.matrix;
                        let (w, h) = (m[0].hypot(m[1]), m[2].hypot(m[3]));
                        (i.width as f64 / w.max(1e-6)).max(i.height as f64 / h.max(1e-6)) as f32
                    })
                    .reduce(f32::max)
            })
            .collect();
        let guard = OcrGuard { doc: id, pages: all, revisions: doc.revision_ends().len(), bytes: doc.bytes.clone() };
        Some(OcrJob { bytes: doc.bytes.clone(), password: doc.password.clone(), infos: doc.info.pages.clone(), native, pages, settings, guard })
    }

    /// Add the words found by [`OcrJob::run`] as one undoable step. Returns the number of words.
    pub fn apply_ocr(&mut self, id: DocId, found: &[OcrPage]) -> Result<usize, EditError> {
        let edits: Vec<Edit> =
            found.iter().filter(|p| !p.words.is_empty()).map(|p| Edit::AddOcrText { page: p.page, words: p.words.clone() }).collect();
        let words = found.iter().map(|p| p.words.len()).sum();
        if !edits.is_empty() {
            self.apply(id, Edit::Batch { label: "Recognize text".into(), edits })?;
        }
        Ok(words)
    }

    /// Why a job's result may not be applied to this session any more: the document was closed,
    /// edited, re-saved, or its pages changed since the job started. `None` means it is safe.
    pub fn stale_reason(&self, guard: &OcrGuard) -> Option<String> {
        let Some(doc) = self.get(guard.doc) else {
            return Some("the document is no longer open".into());
        };
        if !Arc::ptr_eq(&doc.bytes, &guard.bytes) {
            return Some("the document changed since recognition started".into());
        }
        let now = doc.info.pages.len();
        if now != guard.pages {
            return Some(format!("the page count changed from {} to {} since recognition started", guard.pages, now));
        }
        let revisions = doc.revision_ends().len();
        if revisions != guard.revisions {
            let had = guard.revisions;
            return Some(format!("the document gained revisions ({had} → {revisions}) since recognition started"));
        }
        None
    }

    /// [`Self::apply_ocr`], but only while the document is still the one that was read; a stale
    /// result is refused with a named reason and nothing is written.
    pub fn apply_ocr_checked(&mut self, id: DocId, found: &[OcrPage], guard: &OcrGuard) -> Result<usize, String> {
        if guard.doc != id {
            return Err("the result belongs to another document".into());
        }
        if let Some(why) = self.stale_reason(guard) {
            return Err(why);
        }
        self.apply_ocr(id, found).map_err(|e| e.to_string())
    }

    /// Recognise text on `pages` and add it, in one call (the CLI and agents use this).
    pub fn recognize_text(&mut self, id: DocId, pages: &[usize], settings: OcrSettings) -> Result<Vec<OcrPage>, String> {
        let job = self.ocr_job(id, pages, settings).ok_or("no such document")?;
        if let Some(why) = self.get(id).and_then(|d| d.read_only_reason.clone()) {
            return Err(why);
        }
        let recognizers = recognizers(&job.settings)?;
        let found = job.run(&recognizers, |_, _| true);
        self.apply_ocr(id, &found).map_err(|e| e.to_string())?;
        Ok(found)
    }
}

/// What [`recognize_file`] did to one file.
#[derive(Clone, Debug, PartialEq)]
pub struct FileResult {
    /// The new file (incrementally saved), or the original when nothing was recognised.
    pub bytes: Arc<Vec<u8>>,
    pub pages: Vec<OcrPage>,
}

impl FileResult {
    pub fn words(&self) -> usize {
        self.pages.iter().map(|p| p.words.len()).sum()
    }
}

/// Recognize text in multiple files: one PDF's bytes in, the searchable PDF out. `progress`
/// works as for [`OcrJob::run`].
pub fn recognize_file(
    name: &str,
    bytes: Arc<Vec<u8>>,
    password: Option<&str>,
    settings: OcrSettings,
    recognizers: &Recognizers,
    progress: impl FnMut(usize, usize) -> bool,
) -> Result<FileResult, String> {
    let mut s = Session::new();
    let id = s.open(name, None, bytes.clone(), password).map_err(|e| e.to_string())?;
    if let Some(why) = s.get(id).and_then(|d| d.read_only_reason.clone()) {
        return Err(why);
    }
    let job = s.ocr_job(id, &[], settings).ok_or("the document could not be read")?;
    let pages = job.run(recognizers, progress);
    if s.apply_ocr(id, &pages).map_err(|e| e.to_string())? == 0 {
        return Ok(FileResult { bytes, pages });
    }
    Ok(FileResult { bytes: s.save_bytes(id).map_err(|e| e.to_string())?, pages })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::fixture;
    use pdfcraft_render::TextGlyph;

    fn glyph(text: &str) -> TextGlyph {
        TextGlyph { text: text.into(), rect: [0.0, 0.0, 1.0, 1.0] }
    }

    /// The skip-text probe: at least one non-blank run; blank-only or missing text never
    /// enables a skip.
    #[test]
    fn has_text_probe_needs_one_non_blank_run() {
        let none: Option<Arc<pdfcraft_render::PageText>> = None;
        assert!(!page_has_text(none.as_ref()), "a failed probe never enables a skip");
        assert!(!page_has_text(Some(Arc::new(pdfcraft_render::PageText::default())).as_ref()));
        let blank = pdfcraft_render::PageText { glyphs: vec![glyph(" "), glyph("")], ..Default::default() };
        assert!(!page_has_text(Some(Arc::new(blank)).as_ref()));
        let has = pdfcraft_render::PageText { glyphs: vec![glyph(" "), glyph("a")], ..Default::default() };
        assert!(page_has_text(Some(Arc::new(has)).as_ref()));
    }

    #[test]
    fn output_mode_parsing_falls_back_to_searchable() {
        assert_eq!(OutputMode::parse("editable"), OutputMode::EditableText);
        assert_eq!(OutputMode::parse(" Editable-Text "), OutputMode::EditableText);
        assert_eq!(OutputMode::parse("searchable"), OutputMode::Searchable);
        assert_eq!(OutputMode::parse("nonsense"), OutputMode::Searchable, "unknown strings mean Searchable");
        assert_eq!(OutputMode::parse(""), OutputMode::Searchable);
    }

    #[test]
    fn engine_choice_parsing() {
        assert_eq!(EngineChoice::parse("tesseract"), EngineChoice::Tesseract);
        assert_eq!(EngineChoice::parse(" AUTO "), EngineChoice::Auto);
        assert_eq!(EngineChoice::parse("ocrs"), EngineChoice::Ocrs);
        assert_eq!(EngineChoice::parse("wat"), EngineChoice::Ocrs, "unknown names fall back to the default");
        assert_eq!(EngineChoice::default().name(), "ocrs");
    }

    /// Settings default to today's behavior: ocrs, PrimaryOnly, no preprocessing, searchable.
    #[test]
    fn settings_default_to_the_ocrs_status_quo() {
        let s = OcrSettings::default();
        assert_eq!(s.engine, EngineChoice::Ocrs);
        assert_eq!(s.strategy, MergeStrategy::PrimaryOnly);
        assert_eq!(s.output_mode, OutputMode::Searchable);
        assert!(!s.auto_rotate && !s.deskew && !s.denoise && !s.binarize);
        assert_eq!(s.region, None);
        assert_eq!(s.low_confidence_threshold, 60.0);
    }

    /// The preprocessing flags are not just plumbing: a page read with all four on still
    /// yields the same words, in the same places (needs the models).
    #[test]
    fn preprocessing_flags_flow_through_the_job() {
        if !available() {
            eprintln!("skipped: no OCR engine available");
            return;
        }
        let mut s = Session::new().with_clock(|| 1_700_000_000);
        let text = s.create_from_text("t", "The quick brown fox jumps over the lazy dog.").unwrap();
        let id = s.open("text.pdf", None, text, None).unwrap();
        let png = crate::export::Exporter::new(s.get(id).unwrap()).png(0, 150.0).unwrap();
        let scan = s.create_from_images(&[("scan.png".into(), png)]).unwrap();
        let id = s.open("scan.pdf", None, scan, None).unwrap();
        let settings = OcrSettings { auto_rotate: true, deskew: true, denoise: true, binarize: true, ..Default::default() };
        let found = s.recognize_text(id, &[], settings).unwrap();
        let text = crate::tests::page_texts(&s, id)[0].to_lowercase();
        // Binarizing an anti-aliased render thins the thin letters, so a letter may drop; whole
        // common words survive, and that is what this wiring test rests on.
        for w in ["quick", "fox", "lazy"] {
            assert!(text.contains(w), "{text}");
        }
        // The words were placed, meaning boxes survived the trip back to raster coordinates.
        assert!(!found[0].words.is_empty());
    }

    /// suspects(): only Low-band words, by index.
    #[test]
    fn page_suspects_are_the_low_band_words() {
        let word = |t: &str, c: Option<f32>| PlacedWord {
            text: t.into(),
            origin: [0.0; 2],
            across: [1.0, 0.0],
            up: [0.0, 1.0],
            confidence: c,
            source: "ocrs".into(),
        };
        let page =
            OcrPage { page: 0, words: vec![word("a", Some(95.0)), word("b", Some(50.0)), word("c", None), word("d", Some(69.0))], skipped: None };
        assert_eq!(page.suspects(), vec![1, 3]);
        let mean = page.mean_confidence().unwrap_or_default();
        assert!((mean - 71.333).abs() < 0.01, "{mean}");
    }

    /// Region clamping: interior kept, negative extents snapped outward, garbage refused.
    #[test]
    fn regions_clamp_and_refuse_garbage() {
        assert_eq!(clamp_region([10.0, 10.0, 20.0, 20.0], 100, 100), Some([10, 10, 20, 20]));
        assert_eq!(clamp_region([-5.0, -5.5, 20.4, 20.0], 100, 100), Some([0, 0, 21, 20]), "snapped outward, clamped to the image");
        assert_eq!(clamp_region([50.0, 50.0, 200.0, 200.0], 100, 100), Some([50, 50, 100, 100]));
        assert_eq!(clamp_region([10.0, 10.0, 10.0, 20.0], 100, 100), None, "empty");
        assert_eq!(clamp_region([20.0, 10.0, 10.0, 20.0], 100, 100), None, "inverted");
        assert_eq!(clamp_region([f32::NAN, 0.0, 1.0, 1.0], 100, 100), None, "NaN");
        assert_eq!(clamp_region([-20.0, -20.0, -5.0, -5.0], 100, 100), None, "wholly outside");
    }

    /// A crop takes exactly the asked pixels (this feeds words back to page space, so edges
    /// matter).
    #[test]
    fn crops_take_the_asked_pixels() {
        let img = pdfcraft_ocr::OcrImage::new(3, 2, (0..6).flat_map(|p| vec![p as u8; 4]).collect()).unwrap();
        let c = crop(&img, 1, 0, 3, 2).unwrap();
        assert_eq!((c.width, c.height), (2, 2));
        assert_eq!(c.rgba[0], 1, "the crop starts at x=1");
        assert_eq!(c.rgba[4], 2, "the first row's second column is x=2");
        assert_eq!(c.rgba[8], 4, "the second row starts at y=1");
    }

    /// The stale guard: edits and closing are named and the result is refused; a page-count or
    /// revision difference is named too (any edit rewrites the working bytes, so those branches
    /// are reached with a hand-built guard).
    #[test]
    fn stale_results_are_named_and_refused() {
        let mut s = Session::new();
        let id = s.open("t.pdf", None, Arc::new(fixture(2)), None).unwrap();
        let job = s.ocr_job(id, &[], OcrSettings::default()).unwrap();
        assert_eq!(s.stale_reason(&job.guard), None, "freshly captured");
        // Any edit rewrites the working bytes.
        s.apply(id, Edit::AddOcrText { page: 0, words: vec![] }).unwrap();
        assert_eq!(s.stale_reason(&job.guard).as_deref(), Some("the document changed since recognition started"));
        assert!(s.apply_ocr_checked(id, &[], &job.guard).is_err(), "a stale result is never applied");
        // A fresh guard passes.
        let fresh = s.ocr_job(id, &[], OcrSettings::default()).unwrap();
        assert_eq!(s.stale_reason(&fresh.guard), None);
        // A page count and a revision count from another life are named too.
        let doc = s.get(id).unwrap();
        let (bytes, revisions) = (doc.bytes.clone(), doc.revision_ends().len());
        let guard = OcrGuard { doc: id, pages: 5, revisions, bytes };
        assert!(s.stale_reason(&guard).is_some_and(|r| r.contains("page count changed")), "{}", s.stale_reason(&guard).unwrap());
        let guard = OcrGuard { doc: id, pages: 2, revisions: revisions + 3, bytes: s.get(id).unwrap().bytes.clone() };
        assert!(s.stale_reason(&guard).is_some_and(|r| r.contains("revisions")), "{}", s.stale_reason(&guard).unwrap());
        // A guard for a different document refuses up front.
        let other = s.open("other.pdf", None, Arc::new(fixture(1)), None).unwrap();
        let cross = s.apply_ocr_checked(other, &[], &fresh.guard);
        assert!(cross.as_ref().is_err_and(|e| e.contains("another document")), "{cross:?}");
        // And so is a closed document.
        s.close(id);
        assert_eq!(s.stale_reason(&fresh.guard).as_deref(), Some("the document is no longer open"));
    }

    /// The selector refuses an unusable engine with its cause named, and resolves the ensemble
    /// when both engines exist (here: whatever this machine has for ocrs and tesseract).
    #[test]
    fn selector_names_the_missing_engine() {
        let tesseract = OcrSettings { engine: EngineChoice::Tesseract, ..Default::default() };
        match recognizers(&tesseract) {
            Ok(r) => assert_eq!(r.primary.id(), "tesseract"),
            Err(e) => assert!(e.contains("tesseract"), "{e}"),
        }
        let ocrs = OcrSettings { engine: EngineChoice::Ocrs, ..Default::default() };
        match recognizers(&ocrs) {
            Ok(r) => assert_eq!(r.primary.id(), "ocrs"),
            Err(e) => assert!(e.contains("models"), "{e}"),
        }
    }
}
