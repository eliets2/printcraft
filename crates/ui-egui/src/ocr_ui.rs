//! Scan & OCR ▸ Recognize text: the Recognize Text dialog (pages, language, output, resolution)
//! and the background run with its progress. Recognition happens on a worker thread; the result
//! is applied as one undoable edit when it is done.

use std::sync::{Arc, Mutex};

use egui::{Align, Layout};
use pdfcraft_engine::DocId;
use pdfcraft_engine::ocr::{EngineChoice, LANGUAGES, MergeStrategy, OcrPage, OcrSettings, OutputMode};

use crate::theme::{self, Tokens};
use crate::{PdfCraftApp, widgets};

/// Which pages to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrPages {
    All,
    Current,
    Range,
}

/// The dialog's choices (kept between runs, as in Acrobat). The Recognize Text dialog, the OCR
/// Verify screen and the batch options all read and write this one draft, and it is what the
/// preferences persist: language, output mode, the four preprocessing flags, engine, strategy
/// and the three skip flags — never review edits, verified flags or the selection.
#[derive(Clone, Debug, PartialEq)]
pub struct OcrDraft {
    pub pages: OcrPages,
    pub from: usize,
    pub to: usize,
    pub language: String,
    /// "Downsample to" resolution.
    pub dpi: u32,
    /// Which engine reads the pages.
    pub engine: EngineChoice,
    /// How two engines' readings combine.
    pub strategy: MergeStrategy,
    /// Preprocessing flags, all off by default.
    pub auto_rotate: bool,
    pub deskew: bool,
    pub denoise: bool,
    pub binarize: bool,
    /// What Accept produces.
    pub output_mode: OutputMode,
    /// Batch: leave files that already contain text alone.
    pub skip_text_files: bool,
    /// Batch: leave pages that already contain text alone.
    pub skip_text_pages: bool,
    /// Batch: read everything, overriding both skips.
    pub force_ocr: bool,
    /// Batch: where the searchable copies go (None: ask each time). Remembered from the last
    /// batch's folder pick, so a repeated export into the same place does not re-ask.
    pub output_folder: Option<String>,
}

impl Default for OcrDraft {
    fn default() -> Self {
        OcrDraft {
            pages: OcrPages::All,
            from: 1,
            to: 1,
            language: "en".into(),
            dpi: 300,
            engine: EngineChoice::default(),
            strategy: MergeStrategy::default(),
            auto_rotate: false,
            deskew: false,
            denoise: false,
            binarize: false,
            output_mode: OutputMode::default(),
            skip_text_files: false,
            skip_text_pages: true,
            force_ocr: false,
            output_folder: None,
        }
    }
}

impl OcrDraft {
    /// The settings a run with these choices uses.
    pub fn settings(&self) -> OcrSettings {
        OcrSettings {
            dpi: self.dpi as f32,
            language: self.language.clone(),
            engine: self.engine,
            strategy: self.strategy,
            auto_rotate: self.auto_rotate,
            deskew: self.deskew,
            denoise: self.denoise,
            binarize: self.binarize,
            output_mode: self.output_mode,
            skip_text_files: self.skip_text_files,
            skip_text_pages: self.skip_text_pages,
            force_ocr: self.force_ocr,
            ..Default::default()
        }
    }

    /// Restore from the preferences JSON (`ocr` object); unknown or malformed values keep the
    /// defaults, like every other restored setting.
    pub fn apply_pref(&mut self, v: &serde_json::Value) {
        if let Some(l) = v["language"].as_str().filter(|l| LANGUAGES.iter().any(|x| x.0 == *l)) {
            self.language = l.to_string();
        }
        if let Some(e) = v["engine"].as_str() {
            self.engine = EngineChoice::parse(e);
        }
        if let Some(s) = v["strategy"].as_str() {
            self.strategy = match s.trim().to_ascii_lowercase().as_str() {
                "confidence" | "confidence-weighted" | "confidence_weighted" => MergeStrategy::ConfidenceWeighted,
                "rover" | "rover-vote" | "vote" => MergeStrategy::RoverVote,
                "primary" | "primary-only" => MergeStrategy::PrimaryOnly,
                _ => MergeStrategy::PrimaryOnly,
            };
        }
        if let Some(m) = v["output_mode"].as_str() {
            self.output_mode = OutputMode::parse(m);
        }
        for (key, flag) in
            [("auto_rotate", &mut self.auto_rotate), ("deskew", &mut self.deskew), ("denoise", &mut self.denoise), ("binarize", &mut self.binarize)]
        {
            if let Some(on) = v[key].as_bool() {
                *flag = on;
            }
        }
        for (key, flag) in
            [("skip_text_files", &mut self.skip_text_files), ("skip_text_pages", &mut self.skip_text_pages), ("force_ocr", &mut self.force_ocr)]
        {
            if let Some(on) = v[key].as_bool() {
                *flag = on;
            }
        }
        if let Some(f) = v["output_folder"].as_str().filter(|f| !f.is_empty()) {
            self.output_folder = Some(f.to_string());
        }
    }

    /// The preferences JSON for `persist`.
    pub fn to_pref(&self) -> serde_json::Value {
        serde_json::json!({
            "language": self.language,
            "engine": self.engine.name(),
            "strategy": match self.strategy {
                MergeStrategy::PrimaryOnly => "primary",
                MergeStrategy::ConfidenceWeighted => "confidence",
                MergeStrategy::RoverVote => "rover",
            },
            "output_mode": self.output_mode.name(),
            "auto_rotate": self.auto_rotate,
            "deskew": self.deskew,
            "denoise": self.denoise,
            "binarize": self.binarize,
            "skip_text_files": self.skip_text_files,
            "skip_text_pages": self.skip_text_pages,
            "force_ocr": self.force_ocr,
            "output_folder": self.output_folder,
        })
    }
}

/// Progress of a run: pages done, total, the result once finished, and a cancel request.
#[derive(Default)]
pub struct OcrProgress {
    pub done: usize,
    pub total: usize,
    pub result: Option<Result<Vec<OcrPage>, String>>,
    pub cancel: bool,
}

/// Recognize text in multiple files: files done, total, and the summary once finished. Cancel
/// is asked for between files: the file in flight finishes, the rest are counted as not run.
#[derive(Default)]
pub struct BatchProgress {
    pub done: usize,
    pub total: usize,
    pub message: Option<String>,
    pub cancel: bool,
}

pub struct OcrRun {
    pub doc: DocId,
    pub progress: Arc<Mutex<OcrProgress>>,
}

pub(crate) fn body(ui: &mut egui::Ui, app: &mut PdfCraftApp, t: &Tokens) -> (bool, bool) {
    let pages = app.active_ids().and_then(|(_, id)| app.session.get(id)).map_or(1, |d| d.info.pages.len().max(1));
    let d = &mut app.ocr_draft;
    d.to = d.to.clamp(1, pages);
    d.from = d.from.clamp(1, d.to);
    ui.label(egui::RichText::new(tl!("Recognize Text")).font(theme::semibold(18.0)));
    ui.add_space(8.0);
    let group = |ui: &mut egui::Ui, title: &str, body: &mut dyn FnMut(&mut egui::Ui)| {
        ui.label(egui::RichText::new(tl!(title)).font(theme::semibold(13.0)));
        egui::Frame::new().fill(t.hover).corner_radius(egui::CornerRadius::same(6)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui);
        });
        ui.add_space(8.0);
    };
    group(ui, "Pages", &mut |ui| {
        ui.radio_value(&mut d.pages, OcrPages::All, tl!("All pages"));
        ui.radio_value(&mut d.pages, OcrPages::Current, tl!("Current page"));
        ui.horizontal(|ui| {
            ui.radio_value(&mut d.pages, OcrPages::Range, tl!("From"));
            let on = d.pages == OcrPages::Range;
            ui.add_enabled(on, egui::DragValue::new(&mut d.from).range(1..=pages));
            ui.label(tl!("to"));
            ui.add_enabled(on, egui::DragValue::new(&mut d.to).range(1..=pages));
        });
    });
    group(ui, "Settings", &mut |ui| {
        egui::Grid::new("ocr-settings").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label(tl!("Document language"));
            let name = LANGUAGES.iter().find(|l| l.0 == d.language).map_or("English", |l| l.1);
            egui::ComboBox::from_id_salt("ocr-language").selected_text(name).width(220.0).show_ui(ui, |ui| {
                for (code, name) in LANGUAGES {
                    ui.selectable_value(&mut d.language, (*code).to_string(), *name);
                }
            });
            ui.end_row();
            ui.label(tl!("Output"));
            let mode_name = match d.output_mode {
                OutputMode::Searchable => tl!("Searchable Image (Exact)"),
                OutputMode::EditableText => tl!("Editable text"),
            };
            egui::ComboBox::from_id_salt("ocr-output").selected_text(mode_name).width(220.0).show_ui(ui, |ui| {
                let s = ui
                    .selectable_label(d.output_mode == OutputMode::Searchable, tl!("Searchable Image (Exact)"))
                    .on_hover_text(tl!("Adds invisible text over each word; the page image is not changed"));
                if s.clicked() {
                    d.output_mode = OutputMode::Searchable;
                }
                let e = ui
                    .selectable_label(d.output_mode == OutputMode::EditableText, tl!("Editable text"))
                    .on_hover_text(tl!("Writes a new document of visible text; the scan is not changed"));
                if e.clicked() {
                    d.output_mode = OutputMode::EditableText;
                }
            });
            ui.end_row();
            ui.label(tl!("Downsample to"));
            egui::ComboBox::from_id_salt("ocr-dpi").selected_text(format!("{} dpi", d.dpi)).width(220.0).show_ui(ui, |ui| {
                for v in [600, 300, 150, 72] {
                    ui.selectable_value(&mut d.dpi, v, format!("{v} dpi"));
                }
            });
            ui.end_row();
        });
    });
    if let Some(why) = pdfcraft_engine::ocr::engine_unavailable_reason(d.engine) {
        ui.label(egui::RichText::new(why).small().color(t.text_muted));
        ui.add_space(6.0);
    }
    ui.add_space(6.0);
    let (mut go, mut cancel) = (false, false);
    let can_run = pdfcraft_engine::ocr::engine_unavailable_reason(d.engine).is_none();
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add_enabled_ui(can_run, |ui| widgets::pill_button(ui, tl!("Recognize text"), true)).inner.clicked() {
                go = true;
            }
            if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
                cancel = true;
            }
        })
    });
    (go, cancel)
}

impl PdfCraftApp {
    /// Recognize text on the pages chosen in the dialog, in the background.
    pub fn start_ocr(&mut self) {
        let Some((vi, id)) = self.active_ids() else { return };
        if self.ocr_run.is_some() {
            self.notify_tr("Text recognition is already running");
            return;
        }
        if let Some(why) = self.session.get(id).and_then(|d| d.read_only_reason.clone()) {
            self.notify(why);
            return;
        }
        let d = &self.ocr_draft;
        let pages: Vec<usize> = match d.pages {
            OcrPages::All => Vec::new(),
            OcrPages::Current => vec![self.views[vi].current],
            OcrPages::Range => (d.from.saturating_sub(1)..d.to).collect(),
        };
        let settings = d.settings();
        let Some(job) = self.session.ocr_job(id, &pages, settings.clone()) else { return };
        let progress = Arc::new(Mutex::new(OcrProgress { total: job.pages.len(), ..Default::default() }));
        let p = progress.clone();
        let work = move || {
            let result = pdfcraft_engine::ocr::recognizers(&settings).map(|r| {
                job.run(&r, |done, total| {
                    let Ok(mut s) = p.lock() else { return false };
                    s.done = done;
                    s.total = total;
                    !s.cancel
                })
            });
            if let Ok(mut s) = p.lock() {
                s.result = Some(result);
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        if self.run_inline {
            work();
        } else {
            std::thread::Builder::new().name("pdfcraft-ocr".into()).spawn(work).ok();
        }
        #[cfg(target_arch = "wasm32")]
        work();
        self.ocr_run = Some(OcrRun { doc: id, progress });
        self.poll_ocr();
    }

    /// Recognize text in each file, writing the searchable copies into a folder the user picks
    /// (the export folder override in tests), under the same names.
    pub fn ocr_files(&mut self, files: Vec<(String, Vec<u8>)>) {
        if self.ocr_batch.is_some() {
            self.notify_tr("Text recognition is already running");
            return;
        }
        // The settings showing now, not when the folder arrives. A remembered folder (the last
        // batch's pick, persisted with the preferences) is reused; otherwise the user is asked.
        let settings = self.ocr_draft.settings();
        #[cfg(not(target_arch = "wasm32"))]
        match self.export_dir_override.clone().or_else(|| self.ocr_draft.output_folder.clone()) {
            Some(d) => self.ocr_files_into(files, settings, d.into()),
            None => {
                let dialog = rfd::AsyncFileDialog::new().set_title(tl!("Choose a folder for the searchable files").to_string());
                self.ask_one(crate::pickers::Ask::Folder(dialog), None, move |app, dir| app.ocr_files_into(files, settings, dir));
            }
        }
        #[cfg(target_arch = "wasm32")]
        self.ocr_files_into(files, settings);
    }

    /// [`Self::ocr_files`] once the folder for the searchable files is known.
    fn ocr_files_into(&mut self, files: Vec<(String, Vec<u8>)>, settings: OcrSettings, #[cfg(not(target_arch = "wasm32"))] dir: std::path::PathBuf) {
        // Another batch may have started while the folder picker was open.
        if self.ocr_batch.is_some() {
            self.notify_tr("Text recognition is already running");
            return;
        }
        // The folder is remembered (the next batch reuses it; the dialog's Clear un-remembers).
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.ocr_draft.output_folder = Some(dir.to_string_lossy().into_owned());
        }
        // Collision pre-flight: a searchable copy never silently overwrites. Every target is
        // checked before the first page is read; a collision stops the batch here.
        #[cfg(not(target_arch = "wasm32"))]
        {
            let hits: Vec<String> = files
                .iter()
                .filter_map(|(name, _)| {
                    let target = dir.join(name);
                    target.exists().then(|| target.to_string_lossy().into_owned())
                })
                .take(5)
                .collect();
            if !hits.is_empty() {
                self.notify_error(crate::i18n::fmt(
                    tl!("The output folder already holds files of these names; nothing was written: {list}"),
                    &[("list", &hits.join(", "))],
                ));
                return;
            }
        }
        // The engines are built (and named if they cannot be) before anything starts.
        let recognizers = match pdfcraft_engine::ocr::recognizers(&settings) {
            Ok(r) => r,
            Err(why) => {
                self.notify_error(why);
                return;
            }
        };
        let progress = Arc::new(Mutex::new(BatchProgress { total: files.len(), ..Default::default() }));
        let p = progress.clone();
        // The summary is written on the worker thread: draw it in the UI's language.
        let lang = crate::i18n::current();
        let work = move || {
            crate::i18n::set_current(lang);
            #[cfg(not(target_arch = "wasm32"))]
            let write = move |name: &str, r: &pdfcraft_engine::ocr::FileResult| {
                crate::editing::write_atomically(&dir.join(name).to_string_lossy(), &r.bytes).map_err(|e| e.to_string())
            };
            #[cfg(target_arch = "wasm32")]
            let write = move |name: &str, r: &pdfcraft_engine::ocr::FileResult| crate::editing::download(name, &r.bytes);
            let msg = run_batch_files(files, settings, &recognizers, &p, write);
            if let Ok(mut s) = p.lock() {
                s.done = s.total;
                s.message = Some(msg);
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        if self.run_inline {
            work();
        } else {
            std::thread::Builder::new().name("pdfcraft-ocr-files".into()).spawn(work).ok();
        }
        #[cfg(target_arch = "wasm32")]
        work();
        self.ocr_batch = Some(progress);
        self.poll_ocr();
    }

    /// Ask the running batch to stop: the file in flight finishes, the rest are counted as not
    /// run.
    pub fn cancel_ocr_batch(&mut self) {
        if let Some(b) = &self.ocr_batch
            && let Ok(mut s) = b.lock()
        {
            s.cancel = true;
        }
    }

    /// Stop a running recognition (the pages read so far are kept).
    pub fn cancel_ocr(&mut self) {
        if let Some(r) = &self.ocr_run
            && let Ok(mut s) = r.progress.lock()
        {
            s.cancel = true;
        }
    }

    /// Show progress; apply the result once the worker is done.
    pub(crate) fn poll_ocr(&mut self) {
        if let Some(b) = self.ocr_batch.clone() {
            let msg = b.lock().ok().map(|mut s| s.message.take().ok_or((s.done, s.total)));
            match msg {
                Some(Ok(m)) => {
                    self.ocr_batch = None;
                    self.notify(m);
                }
                Some(Err((done, total))) => {
                    let m = crate::i18n::fmt(
                        tl!("Recognizing text… file {d} of {t}"),
                        &[("d", &(done + 1).min(total.max(1)).to_string()), ("t", &total.max(1).to_string())],
                    );
                    if self.toast.as_ref().is_none_or(|t| t.0 != m) {
                        self.notify(m);
                    }
                    if let Some(ctx) = &self.ctx {
                        ctx.request_repaint_after(std::time::Duration::from_millis(200));
                    }
                }
                None => self.ocr_batch = None,
            }
        }
        let Some(run) = self.ocr_run.as_ref() else { return };
        let (doc, progress) = (run.doc, run.progress.clone());
        let Ok(mut s) = progress.lock() else { return };
        let Some(result) = s.result.take() else {
            let msg = crate::i18n::fmt(
                tl!("Recognizing text… page {d} of {t}"),
                &[("d", &(s.done + 1).min(s.total.max(1)).to_string()), ("t", &s.total.max(1).to_string())],
            );
            drop(s);
            if self.toast.as_ref().is_none_or(|t| t.0 != msg) {
                self.notify(msg);
            }
            if let Some(ctx) = &self.ctx {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
            return;
        };
        drop(s);
        self.ocr_run = None;
        let found = match result {
            Ok(f) => f,
            Err(e) => {
                self.notify_error(e);
                return;
            }
        };
        let read = found.iter().filter(|p| p.skipped.is_none()).count();
        let skipped = found.len() - read;
        match self.session.apply_ocr(doc, &found) {
            Ok(words) => {
                if let Some(info) = self.session.get(doc).map(|d| d.info.clone())
                    && let Some(view) = self.views.iter_mut().find(|v| v.id == doc)
                {
                    view.document_changed(&info);
                }
                let words_part = if words == 1 {
                    crate::i18n::fmt(tl!("Recognized 1 word"), &[])
                } else {
                    crate::i18n::fmt(tl!("Recognized {n} words"), &[("n", &words.to_string())])
                };
                let pages_part = if read == 1 { tl!("1 page").to_string() } else { crate::i18n::fmt(tl!("{n} pages"), &[("n", &read.to_string())]) };
                let mut msg = crate::i18n::fmt(tl!("{words} on {pages}"), &[("words", &words_part), ("pages", &pages_part)]);
                if skipped > 0 {
                    msg.push_str(&if skipped == 1 {
                        crate::i18n::fmt(tl!("; 1 page already had text"), &[])
                    } else {
                        crate::i18n::fmt(tl!("; {n} pages already had text"), &[("n", &skipped.to_string())])
                    });
                }
                self.notify(msg);
            }
            Err(e) => self.notify_error(e),
        }
    }
}

/// The batch's worker: read each file, hand the searchable copy to `write` (the output folder,
/// or the web's download), count the buckets, and compose the summary. A file the skip-files
/// option passes over is its own bucket and `write` is never called for it. A cancel is asked
/// for between files: the file in flight finishes, the rest are counted as not run.
pub(crate) fn run_batch_files(
    files: Vec<(String, Vec<u8>)>,
    settings: OcrSettings,
    recognizers: &pdfcraft_engine::ocr::Recognizers,
    progress: &Mutex<BatchProgress>,
    mut write: impl FnMut(&str, &pdfcraft_engine::ocr::FileResult) -> Result<(), String>,
) -> String {
    let total = files.len();
    let (mut ok, mut skipped, mut words, mut failed, mut notes) = (0usize, 0usize, 0usize, Vec::new(), Vec::new());
    let mut not_run: Option<usize> = None;
    for (i, (name, bytes)) in files.into_iter().enumerate() {
        if progress.lock().map(|s| s.cancel).unwrap_or(false) {
            not_run = Some(total - i);
            break;
        }
        if let Ok(mut s) = progress.lock() {
            s.done = i;
        }
        let r = pdfcraft_engine::ocr::recognize_file(&name, Arc::new(bytes), None, settings.clone(), recognizers, |_, _| true);
        match r {
            Ok(res) if res.skipped() => skipped += 1,
            Ok(res) => match write(&name, &res) {
                Ok(()) => {
                    ok += 1;
                    words += res.words();
                    // The batch's low-confidence note: at or under 60, on 1-based sorted pages.
                    if let Some((count, pages)) = pdfcraft_engine::ocr::low_confidence_pages(&res.pages, 60.0) {
                        let list = pages.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
                        notes.push(crate::i18n::fmt(
                            tl!("{name}: {n} low-confidence word(s) on pages {pages}"),
                            &[("name", &name), ("n", &count.to_string()), ("pages", &list)],
                        ));
                    }
                }
                Err(e) => failed.push(format!("{name}: {e}")),
            },
            Err(e) => failed.push(format!("{name}: {e}")),
        }
    }
    batch_summary(ok, words, skipped, &failed, &notes, not_run)
}

/// The batch summary: what was read, the skipped files as their own bucket, the failures, the
/// low-confidence notes and (on a cancel) how many files were not run.
fn batch_summary(ok: usize, words: usize, skipped: usize, failed: &[String], notes: &[String], not_run: Option<usize>) -> String {
    let mut msg = if ok == 1 {
        crate::i18n::fmt(tl!("Recognized {w} words in 1 file"), &[("w", &words.to_string())])
    } else {
        crate::i18n::fmt(tl!("Recognized {w} words in {n} files"), &[("w", &words.to_string()), ("n", &ok.to_string())])
    };
    if skipped > 0 {
        msg.push_str(&crate::i18n::fmt(tl!("; {n} files already had text"), &[("n", &skipped.to_string())]));
    }
    if !failed.is_empty() {
        msg.push_str(&crate::i18n::fmt(tl!("; failed: {list}"), &[("list", &failed.join("; "))]));
    }
    if let Some(rest) = not_run {
        msg.push_str(&crate::i18n::fmt(tl!("; cancelled — {n} files were not run"), &[("n", &rest.to_string())]));
    }
    for note in notes {
        msg.push_str("; ");
        msg.push_str(note);
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdfcraft_engine::ocr::{Recognizer, Recognizers};

    /// A fake engine: one confident word per call, no filesystem, no models needed.
    struct Fake {
        confidence: f32,
    }

    impl Recognizer for Fake {
        fn id(&self) -> &'static str {
            "fake"
        }
        fn available(&self) -> bool {
            true
        }
        fn languages(&self) -> Vec<(String, String)> {
            vec![("en".into(), "English".into())]
        }
        fn recognize(
            &self,
            _image: &pdfcraft_ocr::OcrImage,
            _options: &pdfcraft_ocr::RecognizeOptions,
        ) -> Result<Vec<pdfcraft_ocr::Line>, pdfcraft_engine::ocr::OcrError> {
            Ok(vec![pdfcraft_ocr::Line {
                words: vec![pdfcraft_ocr::Word::new("word", [1.0, 1.0, 9.0, 9.0], "fake").with_confidence(self.confidence)],
            }])
        }
    }

    /// A batch over two text files (the engine never runs on them: they are skipped files):
    /// the summary counts them in their own bucket, nothing is written, and a cancel before
    /// the first file leaves the rest not run.
    #[test]
    fn batch_counts_skipped_files_and_honours_a_cancel() {
        let s = pdfcraft_engine::Session::new();
        let pdf = s.create_from_text("t", "Already typed here").expect("fixture");
        let files = vec![("a.pdf".into(), pdf.to_vec()), ("b.pdf".into(), pdf.to_vec())];
        let progress = Mutex::new(BatchProgress { total: files.len(), cancel: true, ..Default::default() });
        let recognizers = Recognizers { primary: Arc::new(Fake { confidence: 95.0 }), secondary: None };
        let writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let w = writes.clone();
        let msg = run_batch_files(files, OcrSettings { skip_text_files: true, ..Default::default() }, &recognizers, &progress, move |_, _| {
            w.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        });
        assert_eq!(writes.load(std::sync::atomic::Ordering::SeqCst), 0, "nothing is written for skipped files, and the cancel came first");
        assert!(msg.contains("cancelled — 2 files were not run"), "{msg}");
        // Without the cancel, the skipped bucket is named and still nothing is written.
        let files = vec![("a.pdf".into(), pdf.to_vec()), ("b.pdf".into(), pdf.to_vec())];
        let progress = Mutex::new(BatchProgress { total: files.len(), ..Default::default() });
        let w = writes.clone();
        let msg = run_batch_files(files, OcrSettings { skip_text_files: true, ..Default::default() }, &recognizers, &progress, move |_, _| {
            w.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        });
        assert_eq!(writes.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(msg.contains("; 2 files already had text"), "{msg}");
        assert!(msg.contains("Recognized 0 words in 0 files"), "{msg}");
    }

    /// A batch over scanned files: the words are counted, and a low-confidence reading lands
    /// in the summary as a note with its 1-based sorted pages.
    #[test]
    fn batch_notes_low_confidence_pages() {
        let s = pdfcraft_engine::Session::new();
        let pdf = s.create_from_text("t", "fake scan").expect("fixture");
        let files = vec![("scan.pdf".into(), pdf.to_vec()), ("clean.pdf".into(), pdf.to_vec())];
        let progress = Mutex::new(BatchProgress { total: files.len(), ..Default::default() });
        let recognizers = Recognizers { primary: Arc::new(Fake { confidence: 40.0 }), secondary: None };
        // The fixtures are text PDFs standing in for scans; the default skip-text-pages option
        // would (correctly) leave them alone, so the batch here reads everything.
        let settings = OcrSettings { skip_text_pages: false, ..Default::default() };
        let msg = run_batch_files(files, settings.clone(), &recognizers, &progress, |_, _| Ok(()));
        assert!(msg.contains("Recognized 2 words in 2 files"), "{msg}");
        assert!(msg.contains("scan.pdf: 1 low-confidence word(s) on pages 1"), "{msg}");
        // The write failing is a per-file failure, named in the summary.
        let files = vec![("scan.pdf".into(), pdf.to_vec())];
        let progress = Mutex::new(BatchProgress { total: files.len(), ..Default::default() });
        let msg = run_batch_files(files, settings, &recognizers, &progress, |_, _| Err("disk full".into()));
        assert!(msg.contains("; failed: scan.pdf: disk full"), "{msg}");
    }

    /// The remembered output folder survives the preferences round trip; an absent or empty
    /// value keeps the default (ask each time).
    #[test]
    fn the_output_folder_is_persisted_with_the_draft() {
        let mut d = OcrDraft::default();
        assert_eq!(d.output_folder, None, "by default the folder is asked for");
        d.output_folder = Some("out".into());
        let mut r = OcrDraft::default();
        r.apply_pref(&d.to_pref());
        assert_eq!(r.output_folder.as_deref(), Some("out"));
        let mut e = OcrDraft::default();
        e.apply_pref(&serde_json::json!({}));
        assert_eq!(e.output_folder, None, "an absent value keeps the default");
        e.output_folder = Some("kept".into());
        e.apply_pref(&serde_json::json!({ "output_folder": "" }));
        assert_eq!(e.output_folder.as_deref(), Some("kept"), "an empty value is not a folder");
    }

    /// A remembered output folder is used without asking, is remembered again after the run,
    /// and its collision pre-flight still stops the batch before the first page is read.
    #[test]
    fn a_remembered_output_folder_is_used_and_still_preflights_collisions() {
        let mut app = PdfCraftApp::new();
        app.run_inline = true;
        let dir = std::env::temp_dir().join(format!("pdfcraft-ocr-folder-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("fixture");
        std::fs::write(dir.join("a.pdf"), b"would be overwritten").expect("fixture");
        app.ocr_draft.output_folder = Some(dir.to_string_lossy().into_owned());
        let s = pdfcraft_engine::Session::new();
        let pdf = s.create_from_text("t", "scanned words here").expect("fixture");
        app.ocr_files(vec![("a.pdf".into(), pdf.to_vec()), ("b.pdf".into(), pdf.to_vec())]);
        assert!(app.ocr_batch.is_none(), "the collision stops the batch before anything is read");
        let toast = app.toast.clone().map(|t| t.0).unwrap_or_default();
        assert!(toast.contains("already holds files of these names"), "{toast}");
        assert!(toast.contains("a.pdf"), "the colliding names are in the error: {toast}");
        assert_eq!(app.ocr_draft.output_folder.as_deref(), Some(dir.to_string_lossy().as_ref()), "the folder stays remembered for the next batch");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
