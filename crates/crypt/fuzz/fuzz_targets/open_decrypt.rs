//! Opens arbitrary bytes as a PDF with the passwords a protected file may carry, then decrypts its
//! streams, once per kind of crypt filter.
//!
//! The password-less open of the stable-Rust fuzz run never reaches the handler's password
//! comparison for a user or owner password, nor the stream decryption that follows it. Reaching
//! both needs a well-formed encrypted file, so the corpus holds encrypted seeds; mutation then
//! distorts the encryption dictionary, the `/ID` and the stream data from a position that opens.
#![no_main]

use std::sync::Arc;

use libfuzzer_sys::fuzz_target;
use pdfcraft_cos::{Document, Object};
use pdfcraft_crypt::{SecurityHandler, StreamKind};

/// Inputs larger than this are skipped; the fuzzer is also started with a matching `-max_len`, so
/// a buffer is bounded before the parser sees it.
const MAX_INPUT: usize = 16 << 20;

/// The passwords a seeded test document uses. Test-only values, never real credentials.
const PASSWORDS: [&str; 3] = ["", "user", "owner"];

/// Streams decrypted per document; one file may hold many thousands.
const MAX_STREAMS: usize = 64;

/// Objects visited per document; one file may hold millions of small ones.
const MAX_OBJECTS: usize = 512;

/// The crypt filter name tried when the document names no filter of its own.
const DEFAULT_CRYPT_FILTER: &[u8] = b"StdCF";

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }
    let bytes = Arc::new(data.to_vec());
    // Each call authenticates with one password, owner tried before user, as viewers do.
    for password in PASSWORDS {
        let Ok(doc) = Document::open_with_password(Arc::clone(&bytes), Some(password)) else {
            continue;
        };
        let Some(handler) = doc.security() else { continue };
        decrypt_streams(&doc, handler);
    }
});

/// Decrypt the first streams of `doc` as a normal stream, as an embedded file and through a named
/// crypt filter. The result is not inspected: the properties fuzzed here are that no input panics
/// and that every decryption terminates.
fn decrypt_streams(doc: &Document, handler: &SecurityHandler) {
    let name = handler.dict().crypt_filters.first().map_or(DEFAULT_CRYPT_FILTER, |(filter, _)| filter.as_slice());
    let mut decrypted = 0;
    for (reference, object) in doc.scan_objects().take(MAX_OBJECTS) {
        let Object::Stream(stream) = &*object else { continue };
        if decrypted >= MAX_STREAMS {
            break;
        }
        decrypted += 1;
        let raw: &[u8] = &stream.raw;
        let (num, generation) = (reference.num, reference.generation);
        let _ = handler.decrypt_stream(num, generation, raw, StreamKind::Normal);
        let _ = handler.decrypt_stream(num, generation, raw, StreamKind::EmbeddedFile);
        let _ = handler.decrypt_stream(num, generation, raw, StreamKind::Named(name));
    }
}
