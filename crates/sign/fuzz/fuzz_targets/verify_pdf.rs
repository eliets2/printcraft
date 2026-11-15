//! Feeds arbitrary bytes to the PDF-level signature verification entry.
//!
//! That entry reaches every parser the crate has: DER and BER, CMS SignedData, X.509
//! certificates, OCSP responses and CRLs from the document security store, and RFC 3161
//! timestamp tokens.
#![no_main]

use std::sync::Arc;

use libfuzzer_sys::fuzz_target;
use pdfcraft_cos::Document;
use pdfcraft_sign::{TrustStore, signatures};

/// The largest input the target opens (16 MiB); larger inputs are skipped.
const MAX_INPUT: usize = 16 << 20;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }
    let Ok(doc) = Document::open(Arc::new(data.to_vec())) else {
        return;
    };
    // An empty trust store and no clock: certificates are judged at each signature's own
    // signing time or trusted timestamp, so the verdict does not depend on when the target
    // runs.
    let _report = signatures(&doc, data, &TrustStore::default());
});
