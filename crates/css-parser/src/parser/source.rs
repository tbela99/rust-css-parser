//! Port of `src/parser/source.ts`.

use crate::parser::linesmap::LineMap;
use crate::printer::sourcemap::SourceMap;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

thread_local! {
    static SOURCE_ID: Cell<u32> = const { Cell::new(0) };
}

/// source file
#[derive(Debug)]
pub struct SourceFile {
    pub id: u32,
    pub file: Option<Rc<str>>,
    pub line_starts: RefCell<LineMap>,
    /// content: shared with the tokenizer (no copy). appended by the stream tokenizer
    pub content: RefCell<Rc<str>>,
    input_source_map: RefCell<Option<Rc<SourceMap>>>,
}

impl SourceFile {
    pub fn new(content: Rc<str>, lines: Vec<u32>, file: Option<Rc<str>>) -> Rc<Self> {
        let id = SOURCE_ID.with(|s| {
            let id = s.get();
            s.set(id + 1);
            id
        });
        Rc::new(Self {
            id,
            file,
            line_starts: RefCell::new(LineMap::new(lines)),
            content: RefCell::new(content),
            input_source_map: RefCell::new(None),
        })
    }

    /// append content
    pub fn append(&self, content: &str) {
        let mut current = self.content.borrow_mut();
        let mut s = String::with_capacity(current.len() + content.len());
        s.push_str(&current);
        s.push_str(content);
        *current = Rc::from(s);
    }

    /// set the input source map (a source map object as json, or a data url / json string)
    pub fn set_input_source_map(&self, input_source_map: Option<&str>) {
        *self.input_source_map.borrow_mut() = input_source_map.and_then(|s| SourceMap::parse(s).ok()).map(Rc::new);
    }

    pub fn set_input_source_map_object(&self, map: Option<SourceMap>) {
        *self.input_source_map.borrow_mut() = map.map(Rc::new);
    }

    pub fn get_input_source_map(&self) -> Option<Rc<SourceMap>> {
        self.input_source_map.borrow().clone()
    }
}
