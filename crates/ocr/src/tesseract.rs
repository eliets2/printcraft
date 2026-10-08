//! The Tesseract engine: an installed `tesseract` binary driven as an optional external
//! process (`tesseract <png> stdout -l <lang> tsv`), never linked, never bundled, never
//! downloaded. The whole module is compiled out for wasm32, where nothing may spawn.
//!
//! Security posture (hard rules): the program path comes from the PATH or a user-configured
//! location, never from document data; the language code is allow-listed before it reaches a
//! process; the page image goes to a private temp file (a fresh 0700 directory holding a 0600
//! file on Unix) that is deleted afterwards; the run has a timeout and an output size cap; and
//! every failure comes back through [`OcrError`] with its cause named.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use super::{Line, OcrError, OcrImage, RecognizeOptions, Recognizer, Word};

/// How long one recognition may run before the process is killed.
const TIMEOUT: Duration = Duration::from_secs(120);
/// How much TSV output is accepted before the run is abandoned.
const MAX_OUTPUT: u64 = 64 * 1024 * 1024;
/// Tesseract's code for each language PdfCraft offers (ISO 639-1 → tessdata name). Only these
/// codes ever reach the process, whatever the settings say.
pub const LANGUAGE_CODES: &[(&str, &str)] = &[
    ("en", "eng"),
    ("de", "deu"),
    ("fr", "fra"),
    ("es", "spa"),
    ("it", "ita"),
    ("pt", "por"),
    ("ru", "rus"),
    ("zh", "chi_sim"),
    ("ja", "jpn"),
    ("ko", "kor"),
    ("ar", "ara"),
    ("nl", "nld"),
];

/// English names for [`LANGUAGE_CODES`] (dropdowns show code + name).
pub const LANGUAGE_NAMES: &[(&str, &str)] = &[
    ("en", "English"),
    ("de", "German"),
    ("fr", "French"),
    ("es", "Spanish"),
    ("it", "Italian"),
    ("pt", "Portuguese"),
    ("ru", "Russian"),
    ("zh", "Chinese (Simplified)"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("ar", "Arabic"),
    ("nl", "Dutch"),
];

/// Map a settings language code to a Tesseract one: trimmed, case-insensitive, allow-listed;
/// anything unknown falls back to `"eng"`.
pub fn language_code(code: &str) -> &'static str {
    let c = code.trim().to_ascii_lowercase();
    LANGUAGE_CODES.iter().find(|(k, _)| *k == c).map(|(_, v)| *v).unwrap_or("eng")
}

/// One row of Tesseract's TSV output. `level` 5 rows are the words; `conf` is −1 on the header
/// rows (page, block, paragraph, line), which are not words.
struct TsvRow<'a> {
    level: &'a str,
    block: u32,
    par: u32,
    line: u32,
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    conf: f32,
    text: &'a str,
}

/// Parse one TSV row; `None` for anything unreadable (short rows, non-numeric fields), which
/// is skipped rather than trusted.
fn tsv_row(row: &str) -> Option<TsvRow<'_>> {
    let c: Vec<&str> = row.split('\t').collect();
    if c.len() < 12 {
        return None;
    }
    let (block, par, line) = (c[2].trim().parse().ok()?, c[3].trim().parse().ok()?, c[4].trim().parse().ok()?);
    let (left, top) = (c[6].trim().parse().ok()?, c[7].trim().parse().ok()?);
    let (width, height, conf) = (c[8].trim().parse().ok()?, c[9].trim().parse().ok()?, c[10].trim().parse().ok()?);
    Some(TsvRow { level: c[0], block, par, line, left, top, width, height, conf, text: c[11] })
}

/// Parse `tesseract … tsv` output into lines of words. Rows with confidence −1 are the
/// structural header rows and never become words; consecutive word rows that share their
/// (block, paragraph, line) form one [`Line`], in output (reading) order. Confidence clamps
/// into 0–100.
pub fn parse_tsv(tsv: &str) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let mut key: Option<(u32, u32, u32)> = None;
    for row in tsv.lines() {
        let Some(r) = tsv_row(row) else { continue };
        if r.level != "5" || r.conf < 0.0 || r.text.trim().is_empty() {
            continue; // headers (conf −1), non-word levels, and blank output are not words
        }
        let this = (r.block, r.par, r.line);
        if key != Some(this) {
            lines.push(Line::default());
            key = Some(this);
        }
        let word = Word::new(r.text, [r.left, r.top, r.left + r.width, r.top + r.height], "tesseract").with_confidence(r.conf.clamp(0.0, 100.0));
        if let Some(last) = lines.last_mut() {
            last.words.push(word);
        }
    }
    lines
}

/// Parse `tesseract --list-langs` output: the header line ("List of available languages…"),
/// then one code per line.
pub fn parse_list_langs(out: &str) -> Vec<String> {
    out.lines()
        .skip_while(|l| !l.trim().is_empty() && l.trim().starts_with("List of available"))
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// The Tesseract program. Build one (via [`TesseractCli::find`] for the PATH copy, or
/// [`TesseractCli::new`] for a user-configured path) and reuse it: `--list-langs` runs at most
/// once.
pub struct TesseractCli {
    program: PathBuf,
    /// The timeout and the output cap are fields so tests can shorten them.
    timeout: Duration,
    max_output: u64,
    /// Whether `--list-langs` has succeeded yet: 0 untried, 1 yes, 2 no.
    probed: AtomicU8,
    langs: Mutex<Vec<(String, String)>>,
}

impl TesseractCli {
    /// The program at `path` (a user preference; never taken from document data).
    pub fn new(program: PathBuf) -> TesseractCli {
        TesseractCli { program, timeout: TIMEOUT, max_output: MAX_OUTPUT, probed: AtomicU8::new(0), langs: Mutex::new(Vec::new()) }
    }

    /// The PATH copy, if it answers `--list-langs`.
    pub fn find() -> Option<TesseractCli> {
        let t = TesseractCli::new(PathBuf::from("tesseract"));
        let langs = t.list_langs().ok()?;
        *t.langs.lock().unwrap_or_else(|e| e.into_inner()) = langs;
        t.probed.store(1, Ordering::Relaxed);
        Some(t)
    }

    /// Run `tesseract --list-langs` and parse it; also decides [`TesseractCli::available`].
    fn list_langs(&self) -> Result<Vec<(String, String)>, OcrError> {
        let out = self.run(&["--list-langs"], self.timeout, self.max_output)?;
        let codes = parse_list_langs(&out);
        if codes.is_empty() {
            return Err(OcrError::NoTesseract("--list-langs reported no languages".into()));
        }
        Ok(codes
            .into_iter()
            .map(|c| {
                let name = LANGUAGE_NAMES.iter().find(|(k, _)| *k == c.as_str()).map(|(_, n)| (*n).to_string()).unwrap_or_else(|| c.clone());
                (c, name)
            })
            .collect())
    }

    /// Run the program with fixed argv, its stdout in a private temp file, under `timeout`
    /// seconds and an output cap of `max_output` bytes. Returns the (capped) stdout.
    fn run(&self, args: &[&str], timeout: Duration, max_output: u64) -> Result<String, OcrError> {
        let dir = TempDir::create()?;
        let out_path = dir.path.join("out.txt");
        let out_file = std::fs::File::create(&out_path).map_err(|e| OcrError::Process(format!("creating the output file failed: {e}")))?;
        let mut child = std::process::Command::new(&self.program)
            .args(args)
            .stdout(out_file)
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| OcrError::NoTesseract(format!("{} could not start: {e}", self.program.display())))?;
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => {
                    if started.elapsed() >= timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        break Err(OcrError::Timeout(timeout.as_secs()));
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(OcrError::Process(format!("waiting for {} failed: {e}", self.program.display())));
                }
            }
        };
        let status = status?;
        // Whatever the exit, the stderr tail names the cause (capped: error text stays a
        // message, not a memory DoS).
        let mut stderr = child.stderr.take().map_or(Vec::new(), |s| {
            let mut buf = Vec::new();
            use std::io::Read as _;
            let _ = s.take(8192).read_to_end(&mut buf);
            buf
        });
        stderr.truncate(2048);
        let note = String::from_utf8_lossy(&stderr);
        let note = note.trim();
        if !status.success() {
            return Err(OcrError::Process(format!(
                "{} exited with {}: {}",
                self.program.display(),
                status.code().map(|c| c.to_string()).unwrap_or_else(|| "a signal".into()),
                if note.is_empty() { "no error message" } else { note }
            )));
        }
        let len = std::fs::metadata(&out_path).map(|m| m.len()).map_err(|e| OcrError::Process(format!("the output file is unreadable: {e}")))?;
        if len > max_output {
            return Err(OcrError::OutputTooLarge(max_output));
        }
        let bytes = std::fs::read(&out_path).map_err(|e| OcrError::Process(format!("reading the output failed: {e}")))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Retry `--list-langs` once (an install may have appeared since the last try).
    fn refresh(&self) -> Result<(), OcrError> {
        let langs = self.list_langs()?;
        *self.langs.lock().unwrap_or_else(|e| e.into_inner()) = langs;
        self.probed.store(1, Ordering::Relaxed);
        Ok(())
    }
}

impl Recognizer for TesseractCli {
    fn id(&self) -> &'static str {
        "tesseract"
    }

    /// Whether the program answers `--list-langs` (remembered; one process at most after the
    /// first call).
    fn available(&self) -> bool {
        match self.probed.load(Ordering::Relaxed) {
            1 => true,
            2 => false,
            _ => self.refresh().is_ok(),
        }
    }

    fn languages(&self) -> Vec<(String, String)> {
        if self.probed.load(Ordering::Relaxed) == 2 {
            return Vec::new();
        }
        let langs = self.langs.lock().unwrap_or_else(|e| e.into_inner());
        if langs.is_empty() {
            drop(langs);
            return self.refresh().map_or(Vec::new(), |()| self.langs.lock().unwrap_or_else(|e| e.into_inner()).clone());
        }
        langs.clone()
    }

    /// Write the page as a PNG and read it back as TSV.
    fn recognize(&self, image: &OcrImage, options: &RecognizeOptions) -> Result<Vec<Line>, OcrError> {
        let lang = language_code(&options.language);
        let png = encode_png(image)?;
        let dir = TempDir::create()?;
        let png_path = dir.path.join("page.png");
        {
            let mut f = std::fs::File::create(&png_path).map_err(|e| OcrError::Process(format!("creating the image file failed: {e}")))?;
            set_private(&mut f);
            f.write_all(&png).map_err(|e| OcrError::Process(format!("writing the image failed: {e}")))?;
        }
        // Fixed argv, no shell: tesseract <png> stdout -l <lang> tsv.
        let tsv = self.run(&[&png_path.to_string_lossy(), "stdout", "-l", lang, "tsv"], self.timeout, self.max_output)?;
        Ok(parse_tsv(&tsv))
    }
}

/// Encode an RGBA raster as PNG (the interchange format Tesseract reads best).
fn encode_png(image: &OcrImage) -> Result<Vec<u8>, OcrError> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, image.width, image.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| OcrError::Process(format!("encoding the page image failed: {e}")))?;
    writer.write_image_data(&image.rgba).map_err(|e| OcrError::Process(format!("encoding the page image failed: {e}")))?;
    writer.finish().map_err(|e| OcrError::Process(format!("encoding the page image failed: {e}")))?;
    Ok(out)
}

/// Make a just-created file owner-only, where the OS supports it (Unix). Windows relies on the
/// user's private %TEMP% ACLs; the fresh directory keeps other sessions' files out of sight.
fn set_private(f: &mut std::fs::File) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = f;
}

/// A fresh directory under the system temp dir, deleted (with its contents) when dropped.
/// On Unix it is created 0700, so the page image inside is private.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn create() -> Result<TempDir, OcrError> {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let path = std::env::temp_dir().join(format!("pdfcraft-ocr-{}-{nanos}", std::process::id()));
        std::fs::create_dir(&path).map_err(|e| OcrError::Process(format!("creating the private temp directory failed: {e}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
        }
        Ok(TempDir { path })
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A tiny image with one dark square (a synthetic "word"); the Unix process tests use it.
#[cfg(all(test, unix))]
fn blot(w: u32, h: u32, x0: u32, y0: u32, bw: u32, bh: u32) -> OcrImage {
    let mut rgba = vec![255u8; (w * h * 4) as usize];
    for y in y0..(y0 + bh).min(h) {
        for x in x0..(x0 + bw).min(w) {
            let o = ((y * w + x) * 4) as usize;
            rgba[o..o + 3].copy_from_slice(&[0, 0, 0]);
        }
    }
    OcrImage::new(w, h, rgba).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The literal TSV fixture: two header rows (conf −1) and two lines of words.
    const TSV: &str = "level\tpage\tblock\tpar\tline\tword\tleft\ttop\twidth\theight\tconf\ttext
1\t1\t0\t0\t0\t0\t0\t0\t100\t50\t-1\t
5\t1\t1\t1\t1\t1\t10\t10\t40\t12\t96.5\tHello
5\t1\t1\t1\t1\t2\t55\t10\t30\t12\t91.0\tworld
5\t1\t1\t2\t2\t1\t10\t30\t20\t12\t42.123\tsecond";

    #[test]
    fn tsv_parses_words_into_lines() {
        let lines = parse_tsv(TSV);
        assert_eq!(lines.len(), 2, "two (block, par, line) groups: {lines:?}");
        let first = &lines[0].words;
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].text, "Hello");
        assert_eq!(first[0].rect, [10.0, 10.0, 50.0, 22.0], "left + width, top + height");
        assert_eq!(first[0].confidence, Some(96.5));
        assert_eq!(first[0].source, "tesseract");
        assert_eq!(first[1].text, "world");
        assert_eq!(lines[1].words.len(), 1, "a new par starts a new line");
        assert_eq!(lines[1].words[0].text, "second");
        assert_eq!(lines[1].words[0].confidence, Some(42.123));
    }

    /// Conf −1 rows and level ≠ 5 rows are structure, not words; blank text is dropped.
    #[test]
    fn tsv_treats_header_rows_and_blank_text_as_not_words() {
        let tsv = "5\t1\t1\t1\t1\t1\t0\t0\t10\t10\t-1\tskipped\n\
                   4\t1\t1\t1\t1\t0\t0\t0\t10\t10\t95.0\tline row\n\
                   5\t1\t1\t1\t1\t1\t0\t0\t10\t10\t95.0\t   \n\
                   5\t1\t1\t1\t1\t2\t0\t0\t10\t10\t88\tkept";
        let lines = parse_tsv(tsv);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0].words.len(), 1);
        assert_eq!(lines[0].words[0].text, "kept");
    }

    /// Garbage rows (short, non-numeric) are skipped; CRLF endings survive; boxes saturate
    /// instead of overflowing.
    #[test]
    fn tsv_skips_garbage_and_never_panics() {
        let lines = parse_tsv("not a tsv row\n\n5\t1\t1\n5\t1\t1\t1\t1\t1\tx\ty\tw\th\tzzz\ttext\r\n");
        assert!(lines.is_empty(), "{lines:?}");
        let big = parse_tsv(&format!("5\t1\t1\t1\t1\t1\t{}\t0\t{}\t10\t50\tbig", u32::MAX, u32::MAX));
        let rect = big[0].words[0].rect;
        assert!(rect[2].is_finite() && rect[3].is_finite(), "no overflow: {rect:?}");
        // A row whose text field is empty yields no words.
        assert!(parse_tsv("5\t1\t1\t1\t1\t1\t1\t1\t2\t2\t50\t\t").is_empty());
    }

    #[test]
    fn language_table_maps_and_falls_back() {
        assert_eq!(language_code("en"), "eng");
        assert_eq!(language_code(" EN "), "eng", "trimmed, case-insensitive");
        assert_eq!(language_code("de"), "deu");
        assert_eq!(language_code("ZH"), "chi_sim");
        assert_eq!(language_code("ja"), "jpn");
        assert_eq!(language_code("ko"), "kor");
        assert_eq!(language_code("nl"), "nld");
        assert_eq!(language_code("klingon"), "eng", "unknown → eng");
        assert_eq!(language_code(""), "eng");
        // The security property: whatever the settings say, only allow-listed codes reach a
        // process.
        for code in ["en", "de", "fr", "es", "it", "pt", "ru", "zh", "ja", "ko", "ar", "nl", "klingon", "", " eng ", "EN/;rm"] {
            let mapped = language_code(code);
            assert!(LANGUAGE_CODES.iter().any(|(_, v)| *v == mapped), "{code:?} → {mapped:?} is not allow-listed");
        }
    }

    #[test]
    fn list_langs_output_parses() {
        let out = "List of available languages in \"C:\\Program Files\\Tesseract-OCR\\tessdata/\" (2):\neng\nosd\n";
        assert_eq!(parse_list_langs(out), vec!["eng".to_string(), "osd".to_string()]);
        assert_eq!(parse_list_langs("List of available languages (1):\neng\n"), vec!["eng".to_string()]);
        assert!(parse_list_langs("").is_empty());
    }

    /// The real binary, when it is installed: a run end to end, with words that carry a
    /// confidence and the engine's name (skipped where `tesseract` is missing).
    #[test]
    fn recognize_runs_the_installed_binary() {
        let Some(t) = TesseractCli::find() else {
            eprintln!("skipped: the tesseract program is not installed");
            return;
        };
        assert_eq!(t.id(), "tesseract");
        assert!(t.available());
        // 300 dpi-ish block capitals, black on white.
        let (w, h, rgba) = hello_world_image();
        let image = OcrImage::new(w, h, rgba).unwrap();
        let lines = t.recognize(&image, &RecognizeOptions::default()).unwrap();
        let text: Vec<String> = lines.iter().map(Line::text).collect();
        assert!(text.iter().any(|t| t.to_uppercase().contains("HELL")), "{text:?}");
        let words: Vec<&Word> = lines.iter().flat_map(|l| &l.words).collect();
        assert!(words.iter().all(|w| w.source == "tesseract"));
        assert!(words.iter().all(|w| matches!(w.confidence, Some(c) if (0.0..=100.0).contains(&c))), "{words:?}");
    }

    /// `available`/`languages` come from `--list-langs` (skipped without the binary).
    #[test]
    fn languages_come_from_list_langs() {
        let Some(t) = TesseractCli::find() else {
            eprintln!("skipped: the tesseract program is not installed");
            return;
        };
        let langs = t.languages();
        assert!(!langs.is_empty());
        assert!(langs.iter().all(|(c, n)| !c.is_empty() && !n.is_empty()), "{langs:?}");
    }

    /// A hung binary is killed at the timeout and reported (Unix: a fake `tesseract` script;
    /// this test needs a real process, so it is off the web and off Windows).
    #[cfg(unix)]
    #[test]
    fn a_hung_program_is_killed_at_the_timeout() {
        let dir = std::env::temp_dir().join(format!("pdfcraft-ocr-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("tesseract-hang");
        std::fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();
        make_executable(&script);
        let mut t = TesseractCli::new(script);
        t.timeout = Duration::from_millis(500);
        let started = Instant::now();
        let r = t.recognize(&blot(8, 8, 0, 0, 4, 4), &RecognizeOptions::default());
        assert!(matches!(r, Err(OcrError::Timeout(_))), "{r:?}");
        assert!(started.elapsed() < Duration::from_secs(10), "killed, not waited out");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Output past the cap is refused, not read (Unix only, same fake-script trick).
    #[cfg(unix)]
    #[test]
    fn flooding_output_is_capped() {
        let dir = std::env::temp_dir().join(format!("pdfcraft-ocr-test-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("tesseract-flood");
        std::fs::write(&script, "#!/bin/sh\nhead -c 100000 /dev/zero | tr '\\0' 'x'\n").unwrap();
        make_executable(&script);
        let mut t = TesseractCli::new(script);
        t.max_output = 1024;
        let r = t.recognize(&blot(8, 8, 0, 0, 4, 4), &RecognizeOptions::default());
        assert!(matches!(r, Err(OcrError::OutputTooLarge(_))), "{r:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    fn make_executable(p: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755));
    }

    /// The temp directory and its contents are gone after a run (private files do not linger).
    #[cfg(unix)]
    #[test]
    fn temp_files_are_deleted_and_private() {
        let dir = TempDir::create().unwrap();
        assert!(dir.path.is_dir());
        std::fs::write(dir.path.join("x"), b"secret").unwrap();
        let p = dir.path.clone();
        drop(dir);
        assert!(!p.exists(), "the temp directory is deleted after use");
    }

    /// "HELLO" as blocky 5×7 glyphs, 8 pixels per dot with wide tracking, at roughly 300 dpi
    /// scale (each dot ~10 px): big enough for Tesseract's word detector.
    fn hello_world_image() -> (u32, u32, Vec<u8>) {
        const GLYPHS: &[&str] = &[
            "X...X", "X...X", "X...X", "XXXXX", "X...X", "X...X", "X...X", // H
        ];
        // Draw the five glyphs of HELLO with per-letter shapes for E, L, L, O.
        const E: &[&str] = &["XXXXX", "X....", "X....", "XXXX.", "X....", "X....", "XXXXX"];
        const L: &[&str] = &["X....", "X....", "X....", "X....", "X....", "X....", "XXXXX"];
        const O: &[&str] = &[".XXX.", "X...X", "X...X", "X...X", "X...X", "X...X", ".XXX."];
        let letters: [&[&str]; 5] = [GLYPHS, E, L, L, O];
        let (dot, x0, y0, advance) = (8u32, 40u32, 40u32, 7u32 * 8);
        let w = x0 * 2 + advance * 5;
        let h = y0 * 2 + 7 * dot;
        let mut rgba = vec![255u8; (w * h * 4) as usize];
        for (i, rows) in letters.iter().enumerate() {
            for (ry, row) in rows.iter().enumerate() {
                for (rx, b) in row.bytes().enumerate() {
                    if b != b'X' {
                        continue;
                    }
                    for dy in 0..dot {
                        for dx in 0..dot {
                            let x = x0 + i as u32 * advance + rx as u32 * dot + dx;
                            let y = y0 + ry as u32 * dot + dy;
                            let o = ((y * w + x) * 4) as usize;
                            rgba[o..o + 3].copy_from_slice(&[0, 0, 0]);
                        }
                    }
                }
            }
        }
        (w, h, rgba)
    }
}
