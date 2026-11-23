//! Port of `src/printer/result.ts` (rendering part, see the host api for the javascript classes).

use crate::printer::sourcemap::SourceMap;
use crate::types::ErrorDescription;

/// `RenderResult`: the rendered css (utf-8 bytes, decoded lazily by the bindings)
#[derive(Clone, Debug)]
pub struct RenderResult {
    pub bytes: Vec<u8>,
    pub errors: Vec<ErrorDescription>,
    pub total: String,
    pub map: Option<SourceMap>,
    /// length of the css in utf-16 code units (`code.length`)
    pub code_length: usize,
}

impl RenderResult {
    /// the rendered css
    pub fn code(&self) -> &str {
        std::str::from_utf8(&self.bytes).unwrap_or_default()
    }

    /// `RenderResult.codeLength()`
    pub fn code_length(&self) -> usize {
        self.code_length
    }
}
