use sysinfo::{System, SystemExt};

#[cfg(target_pointer_width = "64")]
pub mod mimalloc_constants {
    pub const MI_SEGMENT_ALIGN: usize = 32 * 1024 * 1024;     // 32 MB
    pub const MI_ARENA_BLOCK_SIZE: usize = 64 * 1024 * 1024;  // 64 MB
}

#[cfg(target_pointer_width = "32")]
pub mod mimalloc_constants {
    pub const MI_SEGMENT_ALIGN: usize = 4 * 1024 * 1024;      // 4 MB
    pub const MI_ARENA_BLOCK_SIZE: usize = 4 * 1024 * 1024;   // 4 MB
}

pub fn get_default_mmap_size() -> usize {
    let mut sys = System::new();
    sys.refresh_memory();

    let free_memory = sys.free_memory();
    let memory_to_map = free_memory / 2;

    return usize::try_from(memory_to_map).unwrap_or(usize::MAX);
}