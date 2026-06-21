use crate::kernel::KernelStatus;

pub(super) fn transient_status_label(status: KernelStatus) -> Option<&'static str> {
    match status {
        KernelStatus::Connecting => Some("Connecting to kernel..."),
        KernelStatus::Busy => Some("Kernel busy. Ctrl-C to interrupt"),
        _ => None,
    }
}
