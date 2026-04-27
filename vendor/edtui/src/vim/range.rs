use jagged::Index2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RangeKind {
    Exclusive,
    Inclusive,
    Linewise,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TextRange {
    pub(crate) start: Index2,
    pub(crate) end: Index2,
    pub(crate) kind: RangeKind,
}

impl TextRange {
    pub(crate) fn exclusive(start: Index2, end: Index2) -> Self {
        Self {
            start,
            end,
            kind: RangeKind::Exclusive,
        }
    }

    pub(crate) fn inclusive(start: Index2, end: Index2) -> Self {
        Self {
            start,
            end,
            kind: RangeKind::Inclusive,
        }
    }

    pub(crate) fn linewise(start_row: usize, end_row: usize) -> Self {
        Self {
            start: Index2::new(start_row, 0),
            end: Index2::new(end_row, 0),
            kind: RangeKind::Linewise,
        }
    }
}
