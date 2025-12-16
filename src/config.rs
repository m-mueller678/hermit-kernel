pub(crate) const KERNEL_STACK_SIZE: usize = 0x8000;

pub const DEFAULT_STACK_SIZE: usize = 0x0001_0000;

pub(crate) const USER_STACK_SIZE: usize = 0x0010_0000;

#[cfg(any(feature = "fuse", feature = "vsock", feature = "console",))]
pub(crate) const VIRTIO_MAX_QUEUE_SIZE: u16 = if cfg!(feature = "pci") { 2048 } else { 1024 };

#[cfg(feature = "vsock")]
pub(crate) const VSOCK_PACKET_SIZE: u32 = 8192;

#[cfg(feature = "console")]
pub(crate) const CONSOLE_PACKET_SIZE: u32 = 8192;
