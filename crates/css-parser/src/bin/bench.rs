//! allocation and time profile of parse / minify / render (`bench <file> [iterations]`)
use rust_css_parser::host::NoHost;
use rust_css_parser::options::{ParserOptions, SourcesMap};
use rust_css_parser::parser::input::CssInput;
use rust_css_parser::parser::parse::do_parse_sync;
use rust_css_parser::parser::source::SourceFile;
use rust_css_parser::parser::tokenize::{create_parse_info, Tokenizer};
use rust_css_parser::printer::render::do_render;
use std::alloc::{GlobalAlloc, Layout, System};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// allocation sites sampled when BENCH_TRACE is set: (stack, count)
    static SITES: std::cell::RefCell<std::collections::HashMap<String, usize>> = std::cell::RefCell::new(std::collections::HashMap::new());
    static IN_TRACE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
static TRACE: AtomicUsize = AtomicUsize::new(0);

/// record the crate frames of the current allocation
fn sample() {
    if IN_TRACE.with(|t| t.replace(true)) {
        return;
    }
    let trace = std::backtrace::Backtrace::force_capture().to_string();
    // the source locations of the crate frames (the inlined frames included)
    let frames: Vec<String> = trace
        .lines()
        .filter_map(|l| l.trim().strip_prefix("at "))
        .filter(|l| (l.starts_with("./src/") || l.contains("/rust-css-parser/src/")) && !l.contains("/bin/"))
        .map(|l| l.trim_start_matches("./").rsplitn(2, ':').nth(1).unwrap_or(l).to_string())
        .take(3)
        .collect();
    if std::env::var("BENCH_TRACE_RAW").is_ok() && frames.is_empty() {
        eprintln!("{}", trace.lines().take(60).collect::<Vec<_>>().join("\n"));
        std::process::exit(0);
    }
    let key = frames.join(" <- ");
    SITES.with(|s| *s.borrow_mut().entry(key).or_default() += 1);
    IN_TRACE.with(|t| t.set(false));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let n = ALLOCS.fetch_add(1, Ordering::Relaxed);
        let every = TRACE.load(Ordering::Relaxed);
        if every != 0 && n % every == 0 {
            sample();
        }
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new_size, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn snapshot() -> (usize, usize) {
    (ALLOCS.load(Ordering::Relaxed), BYTES.load(Ordering::Relaxed))
}

fn main() {
    let path = std::env::args().nth(1).expect("file");
    let iterations: usize = std::env::args().nth(2).and_then(|n| n.parse().ok()).unwrap_or(1);
    let bytes = std::fs::read(&path).expect("read");
    if let Ok(every) = std::env::var("BENCH_TRACE") {
        TRACE.store(every.parse().unwrap_or(97), Ordering::Relaxed);
        std::panic::set_hook(Box::new(|_| {}));
        let _ = std::panic::take_hook();
    }
    if std::env::var("BENCH_RENDER").is_ok() {
        // render only: the non minified ast is rendered `iterations` times
        let source = SourceFile::new(Rc::from(""), Vec::new(), Some(Rc::from("")));
        let sources = SourcesMap::default();
        sources.borrow_mut().insert(source.id, source.clone());
        let options = ParserOptions { minify: Some(false), source: Some(source.clone()), sources_map: Some(sources), ..Default::default() };
        let tokenizer = Tokenizer::new(create_parse_info(Some(&CssInput::from(bytes.clone())), source));
        let result = do_parse_sync(tokenizer, options, None, Rc::new(NoHost)).expect("parse");
        let t0 = Instant::now();
        for _ in 0..iterations {
            let out = do_render(&result.arena, result.ast, &ParserOptions { minify: Some(true), ..Default::default() }, None);
            std::hint::black_box(out.bytes.len());
        }
        println!("render {:.2} ms", (Instant::now() - t0).as_secs_f64() * 1000.0 / iterations as f64);
        return;
    }
    // BENCH_MODE=minify / fast: only one of the modes
    let modes: Vec<bool> = match std::env::var("BENCH_MODE").as_deref() {
        Ok("minify") => vec![true],
        Ok("fast") => vec![false],
        _ => vec![false, true],
    };
    for minify in modes {
        let mut stats = [(0usize, 0usize, 0f64); 2];
        // the fastest run: less sensitive to the noise of the machine than the mean
        let mut best = [f64::INFINITY; 2];
        for _ in 0..iterations {
            let source = SourceFile::new(Rc::from(""), Vec::new(), Some(Rc::from("")));
            let sources = SourcesMap::default();
            sources.borrow_mut().insert(source.id, source.clone());
            let options = ParserOptions { minify: Some(minify), source: Some(source.clone()), sources_map: Some(sources), ..Default::default() };
            let (a0, b0) = snapshot();
            let t0 = Instant::now();
            let tokenizer = Tokenizer::new(create_parse_info(Some(&CssInput::from(bytes.clone())), source));
            let result = do_parse_sync(tokenizer, options, None, Rc::new(NoHost)).expect("parse");
            let t1 = Instant::now();
            let (a1, b1) = snapshot();
            let out = do_render(&result.arena, result.ast, &ParserOptions { minify: Some(true), ..Default::default() }, None);
            let t2 = Instant::now();
            let (a2, b2) = snapshot();
            std::hint::black_box(out.bytes.len());
            stats[0].0 += a1 - a0;
            stats[0].1 += b1 - b0;
            stats[0].2 += (t1 - t0).as_secs_f64() * 1000.0;
            best[0] = best[0].min((t1 - t0).as_secs_f64() * 1000.0);
            best[1] = best[1].min((t2 - t1).as_secs_f64() * 1000.0);
            stats[1].0 += a2 - a1;
            stats[1].1 += b2 - b1;
            stats[1].2 += (t2 - t1).as_secs_f64() * 1000.0;
        }
        let n = iterations as f64;
        println!(
            "minify={:5} parse: {:8.0} allocs {:8.0} KB {:7.2} ms (min {:7.2}) | render: {:8.0} allocs {:8.0} KB {:7.2} ms (min {:6.2})",
            minify,
            stats[0].0 as f64 / n,
            stats[0].1 as f64 / n / 1024.0,
            stats[0].2 / n,
            best[0],
            stats[1].0 as f64 / n,
            stats[1].1 as f64 / n / 1024.0,
            stats[1].2 / n,
            best[1]
        );
    }
    if TRACE.load(Ordering::Relaxed) != 0 {
        TRACE.store(0, Ordering::Relaxed);
        let mut sites: Vec<(String, usize)> = SITES.with(|s| s.borrow().iter().map(|(k, v)| (k.clone(), *v)).collect());
        sites.sort_by(|a, b| b.1.cmp(&a.1));
        for (site, count) in sites.iter().take(40) {
            println!("{:6} {}", count, site);
        }
    }
}
