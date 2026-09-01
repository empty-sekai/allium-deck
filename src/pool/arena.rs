use std::alloc::{Layout, alloc_zeroed, dealloc, handle_alloc_error};
use std::ptr::NonNull;

/// 64 字节对齐的连续内存块。
#[derive(Debug)]
pub(crate) struct Arena {
    ptr: NonNull<u8>,
    layout: Layout,
}

// Arena 拥有整块分配内存；freeze 成 CardPool 后只通过共享引用读取列数据。
// PoolBuilder 构建阶段独占持有 Arena，可变写入不会跨线程共享。
unsafe impl Send for Arena {}
unsafe impl Sync for Arena {}

impl Arena {
    pub(crate) fn new(size: usize) -> Self {
        assert!(size > 0, "arena size must be greater than zero");
        let layout = match Layout::from_size_align(size, 64) {
            Ok(layout) => layout,
            Err(_) => panic!("invalid arena layout"),
        };
        let ptr = unsafe { alloc_zeroed(layout) };
        if ptr.is_null() {
            handle_alloc_error(layout);
        }

        let ptr = unsafe { NonNull::new_unchecked(ptr) };
        Self { ptr, layout }
    }

    #[inline(always)]
    pub(crate) const fn as_ptr(&self) -> *const u8 {
        self.ptr.as_ptr() as *const u8
    }

    #[inline(always)]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr.as_ptr()
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.ptr.as_ptr(), self.layout);
        }
    }
}
