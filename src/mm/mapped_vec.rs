use core::ops::Deref;
use std::{iter::FromIterator, ptr::NonNull, slice};

use crate::mm::memory_domain::{alloc_from_domain, free_to_domain, realloc_in_domain};

#[derive(Debug)]
pub struct MappedVec<T> {
    ptr: NonNull<T>,
    len: usize,
    cap: usize,
}

unsafe impl<T: Send> Send for MappedVec<T> {}
unsafe impl<T: Sync> Sync for MappedVec<T> {}

impl<T> Drop for MappedVec<T> {
    fn drop(&mut self) {
        if self.cap != 0 {
            free_to_domain(self.ptr);
        }
    }
}

impl<T> Deref for MappedVec<T> {
    type Target = [T];

    #[inline]
    fn deref(&self) -> &[T] {
        unsafe { slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl<T> AsRef<[T]> for MappedVec<T> {
    #[inline]
    fn as_ref(&self) -> &[T] {
        self
    }
}

impl<T> MappedVec<T> {
    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.ptr.as_ptr()
    }

    #[inline]
    pub fn set_len(&mut self, new_len: usize) {
        debug_assert!(new_len <= self.cap);
        self.len = new_len;
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.cap
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn with_capacity(capacity: usize) -> MappedVec<T> {
        let (ptr, cap) = alloc_from_domain::<T>(capacity);
        MappedVec { ptr, len: 0, cap }
    }

    pub unsafe fn from_raw_parts(ptr: *mut T, len: usize, cap: usize) -> MappedVec<T> {
        MappedVec {
            ptr: NonNull::new_unchecked(ptr),
            len,
            cap,
        }
    }

    pub fn shrink_to_fit(&mut self) {
        if self.cap > self.len {
            if self.len == 0 {
                self.ptr = NonNull::dangling();
                self.cap = 0;
            } else {
                let (new_ptr, new_cap) = realloc_in_domain(self.ptr, self.len);
                self.ptr = new_ptr;
                self.cap = new_cap;
            }
        }
    }

    pub fn reserve(&mut self, additional: usize) {
        let required = self.len.checked_add(additional).expect("capacity overflow");
        if required <= self.cap {
            return;
        }
        
        let mut new_cap = self.cap * 2;
        if new_cap < required {
            new_cap = required;
        }

        let (new_ptr, allocated_cap) = if self.cap == 0 {
            alloc_from_domain(new_cap)
        } else {
            realloc_in_domain(self.ptr, new_cap)
        };
        self.ptr = new_ptr;
        self.cap = allocated_cap;
    }

    pub fn push(&mut self, value: T) {
        if self.len == self.cap {
            self.reserve(1);
        }
        unsafe {
            std::ptr::write(self.ptr.as_ptr().add(self.len), value);
        }
        self.len += 1;
    }
}

impl<T: Copy> MappedVec<T> {
    pub fn extend_from_slice(&mut self, other: &[T]) {
        if other.is_empty() {
            return;
        }
        self.reserve(other.len());
        unsafe {
            let dst = self.ptr.as_ptr().add(self.len);
            std::ptr::copy(other.as_ptr(), dst, other.len());
        }
        self.len += other.len();
    }
}

impl<T> FromIterator<T> for MappedVec<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let iter = iter.into_iter();
        let (lower, upper) = iter.size_hint();
        let initial_cap = upper.unwrap_or(lower);

        let mut vec = MappedVec::with_capacity(initial_cap);
        for item in iter {
            vec.push(item);
        }
        vec
    }
}

impl<T> Default for MappedVec<T> {
    fn default() -> Self {
        MappedVec::with_capacity(0)
    }
}

