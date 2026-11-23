//! Port of `src/parser/linesmap.ts`.

/// compute the line and column of an offset
#[derive(Clone, Debug)]
pub struct LineMap {
    pub line_starts: Vec<u32>,
}

impl LineMap {
    pub fn new(mut lines: Vec<u32>) -> Self {
        if lines.is_empty() {
            lines.push(0);
        }
        Self { line_starts: lines }
    }

    /// `[line, column]` of an offset (1-based line, 1-based column on the first line)
    pub fn get_offsets(&self, offset: u32) -> (u32, u32) {
        let line = self.search(offset);
        if line < 0 {
            // typescript: lineStarts[-1] is undefined -> NaN
            return (0, 0);
        }
        let column = offset as i64 - self.line_starts[line as usize] as i64;
        (line as u32 + 1, if line == 0 { (column + 1) as u32 } else { column as u32 })
    }

    /// greatest index of a line start less than or equal to the offset, -1 when none
    pub fn search(&self, offset: u32) -> i64 {
        // `partition_point` is the binary search of the typescript implementation
        self.line_starts.partition_point(|&start| start <= offset) as i64 - 1
    }
}
