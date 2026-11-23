//! `performance.now()`: milliseconds. native: a monotonic clock, wasm: the host clock (imported `css_now`)

#[cfg(not(target_arch = "wasm32"))]
pub fn now() -> f64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

#[cfg(target_arch = "wasm32")]
pub fn now() -> f64 {
    #[link(wasm_import_module = "env")]
    extern "C" {
        fn css_now() -> f64;
    }
    // SAFETY: the host provides `css_now`
    unsafe { css_now() }
}

/// `${ms.toFixed(2)}ms`
pub fn format_ms(ms: f64) -> String {
    format!("{}ms", crate::utils::number::to_fixed(ms, 2))
}
