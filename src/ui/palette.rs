#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PaletteAction {
    Quit,
    InterruptKernel,
    RestartKernel,
    ClearInput,
    ShowConnectionInfo,
}

impl PaletteAction {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Quit => "Quit",
            Self::InterruptKernel => "Interrupt Kernel",
            Self::RestartKernel => "Restart Kernel",
            Self::ClearInput => "Clear Input",
            Self::ShowConnectionInfo => "Show Connection Info",
        }
    }
}

pub(super) fn palette_items() -> [PaletteAction; 5] {
    [
        PaletteAction::Quit,
        PaletteAction::InterruptKernel,
        PaletteAction::RestartKernel,
        PaletteAction::ClearInput,
        PaletteAction::ShowConnectionInfo,
    ]
}
