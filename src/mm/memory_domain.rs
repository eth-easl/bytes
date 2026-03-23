use libmimalloc_sys::{
    mi_arena_id_t, mi_free, mi_heap_malloc_aligned, mi_heap_new_in_arena, mi_heap_realloc_aligned,
    mi_heap_t, mi_manage_os_memory_ex, mi_usable_size,
};
use nix::{
    sys::{
        memfd::{memfd_create, MemFdCreateFlag},
        mman::{MapFlags, ProtFlags},
    },
    unistd::ftruncate,
};
use std::{
    cell::RefCell, env, ffi::CString, format, io::{Error, ErrorKind, Result}, num::NonZeroUsize, os::{fd::RawFd, raw::c_void}, println, ptr::NonNull, str::FromStr, sync::LazyLock, thread_local
};

use crate::mm::constants::mimalloc_constants::{MI_ARENA_BLOCK_SIZE, MI_SEGMENT_ALIGN};

pub struct MemoryDomain {
    pub fd: RawFd,
    pub arena_id: mi_arena_id_t,
    pub base_ptr: usize,
    pub size: usize,
}

impl MemoryDomain {
    pub fn init() -> Result<MemoryDomain> {
        // setting the default size of the memory domain to 1GB
        let mut mm_size_bytes = env::var("MM_SIZE_BYTES")
            .map_err(|e| Error::new(ErrorKind::NotFound, format!("MM_SIZE_BYTES missing: {}", e)))?
            .parse::<usize>()
            .map_err(|e| {
                Error::new(
                    ErrorKind::InvalidData,
                    format!("MM_SIZE_BYTES parse error: {}", e),
                )
            })?;

        // Round down to nearest multiple of MI_SEGMENT_ALIGN
        mm_size_bytes = mm_size_bytes & (!(MI_SEGMENT_ALIGN - 1));
        if mm_size_bytes < MI_ARENA_BLOCK_SIZE {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("MM_SIZE_BYTES must be at least {}", MI_ARENA_BLOCK_SIZE),
            ));
        }

        let fd = memfd_create(
            &CString::from_str("bytes_memory_domain").unwrap(),
            MemFdCreateFlag::empty(),
        )
        .unwrap();

        ftruncate(fd, i64::try_from(mm_size_bytes).unwrap())?;

        let aligned_ptr = unsafe {
            let reserved_size = mm_size_bytes + MI_SEGMENT_ALIGN;
            let reserve_ptr = nix::sys::mman::mmap(
                None,
                NonZeroUsize::new(reserved_size).unwrap(),
                ProtFlags::PROT_NONE,
                MapFlags::MAP_PRIVATE | MapFlags::MAP_ANONYMOUS,
                -1,
                0,
            )
            .or(Err(Error::new(
                ErrorKind::Other,
                "Failed to reserve virtual address space",
            )))?;

            // address passed to mimalloc must be MI_SEGMENT_ALIGN aligned
            let aligned_addr =
                (reserve_ptr as usize + MI_SEGMENT_ALIGN - 1) & !(MI_SEGMENT_ALIGN - 1);

            let base_ptr = nix::sys::mman::mmap(
                NonZeroUsize::new(aligned_addr),
                NonZeroUsize::new(mm_size_bytes).unwrap(),
                ProtFlags::all(),
                MapFlags::MAP_SHARED | MapFlags::MAP_FIXED,
                fd,
                0,
            )
            .or(Err(Error::new(ErrorKind::Other, "Failed to mmap memory")))?;

            // unmap remaining space
            let prefix_size = aligned_addr - reserve_ptr as usize;
            if prefix_size > 0 {
                let _ = nix::sys::mman::munmap(reserve_ptr, prefix_size);
            }

            let suffix_size = reserved_size - prefix_size - mm_size_bytes;
            if suffix_size > 0 {
                let _ = nix::sys::mman::munmap(
                    (aligned_addr + mm_size_bytes) as *mut c_void,
                    suffix_size,
                );
            }

            base_ptr
        };

        let mut arena_id: mi_arena_id_t = 0;
        let success = unsafe {
            mi_manage_os_memory_ex(
                aligned_ptr,
                mm_size_bytes,
                true,
                false,
                true,
                -1,
                true,
                &mut arena_id,
            )
        };

        if !success {
            return Err(Error::new(
                ErrorKind::Other,
                "Failed to register aligned memory with mimalloc Arena",
            ));
        }

        Ok(MemoryDomain {
            fd,
            arena_id,
            base_ptr: aligned_ptr as usize,
            size: mm_size_bytes
        })
    }
}

pub static MAPPED_MEMORY_DOMAIN: LazyLock<MemoryDomain> =
    LazyLock::new(|| MemoryDomain::init().expect("Failed to initialize memory domain"));

thread_local! {
    static LOCAL_HEAP: RefCell<*mut mi_heap_t> = RefCell::new(
        unsafe { mi_heap_new_in_arena(MAPPED_MEMORY_DOMAIN.arena_id) }
    );
}

pub fn alloc_from_domain<T>(size: usize) -> (NonNull<T>, usize) {
    if size == 0 || size_of::<T>() == 0 {
        return (NonNull::dangling(), size);
    }

    LOCAL_HEAP.with(|heap_ptr| {
        let alignment = align_of::<T>();
        let raw = unsafe { mi_heap_malloc_aligned(*heap_ptr.borrow_mut(), size, alignment) };
        let ptr = NonNull::new(raw as *mut T).expect("OOM during alloc");
        let true_capacity = unsafe { mi_usable_size(raw) };
        (ptr, true_capacity)
    })
}

pub fn realloc_in_domain<T>(ptr: NonNull<T>, new_size: usize) -> (NonNull<T>, usize) {
    if new_size == 0 || size_of::<T>() == 0 {
        return (NonNull::dangling(), new_size);
    }

    LOCAL_HEAP.with(|heap_ptr| {
        let alignment = align_of::<T>();
        let new_raw = unsafe {
            mi_heap_realloc_aligned(
                *heap_ptr.borrow_mut(),
                ptr.as_ptr() as *mut c_void,
                new_size,
                alignment,
            )
        };
        let new_ptr = NonNull::new(new_raw as *mut T).expect("OOM during realloc");
        let new_capacity = unsafe { mi_usable_size(new_raw) };
        (new_ptr, new_capacity)
    })
}

pub fn free_to_domain<T>(ptr: NonNull<T>) {
    unsafe {
        mi_free(ptr.as_ptr() as *mut c_void);
    }
}

pub fn get_mmap_details() -> (RawFd, usize, usize) {
    (MAPPED_MEMORY_DOMAIN.fd, MAPPED_MEMORY_DOMAIN.base_ptr, MAPPED_MEMORY_DOMAIN.size)
}
