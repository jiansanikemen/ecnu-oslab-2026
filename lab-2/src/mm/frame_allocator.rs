use core::ptr::{read_volatile, write_bytes, write_volatile};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::{PhysAddr, PhysPageNum, PAGE_SIZE};

static FRAME_LOCK: AtomicBool = AtomicBool::new(false);
static HEAD: AtomicUsize = AtomicUsize::new(0);
static START_PPN: AtomicUsize = AtomicUsize::new(0);
static END_PPN: AtomicUsize = AtomicUsize::new(0);

fn lock() {
    while FRAME_LOCK.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
}

fn unlock() {
    FRAME_LOCK.store(false, Ordering::Release);
}

fn total_frames() -> usize {
    END_PPN
        .load(Ordering::Relaxed)
        .saturating_sub(START_PPN.load(Ordering::Relaxed))
}

pub struct FrameTracker {
    ppn: PhysPageNum,
}

impl FrameTracker {
    pub fn ppn(&self) -> PhysPageNum {
        self.ppn
    }
}

impl Drop for FrameTracker {
    fn drop(&mut self) {
        let ppn = self.ppn.0;
        let start = START_PPN.load(Ordering::Acquire);
        let end = END_PPN.load(Ordering::Acquire);

        if ppn < start || ppn >= end {
            panic!("frame released outside managed range");
        }

        let page_addr = ppn << 12;
        lock();

        let mut current = HEAD.load(Ordering::Relaxed);
        let mut steps = 0;

        while current != 0 {
            if current & (PAGE_SIZE - 1) != 0
                || current >> 12 < start
                || current >> 12 >= end
                || steps >= total_frames()
            {
                unlock();
                panic!("free-frame list is corrupted");
            }

            if current == page_addr {
                unlock();
                panic!("frame released twice");
            }

            current = unsafe { read_volatile(current as *const usize) };
            steps += 1;
        }

        unsafe {
            write_volatile(page_addr as *mut usize, HEAD.load(Ordering::Relaxed));
        }
        HEAD.store(page_addr, Ordering::Relaxed);
        unlock();
    }
}

/// # Safety
/// `kernel_end` 之后到 `memory_end` 的地址必须是可写、页对齐后可分配的 RAM。
pub unsafe fn frame_allocator_init(kernel_end: PhysAddr, memory_end: PhysAddr) {
    let start = kernel_end.ceil().0;
    let end = memory_end.floor().0;

    assert!(start < end);

    lock();
    START_PPN.store(start, Ordering::Relaxed);
    END_PPN.store(end, Ordering::Relaxed);
    HEAD.store(0, Ordering::Relaxed);

    for ppn in start..end {
        let page_addr = PhysPageNum(ppn).addr().0;

        unsafe {
            write_volatile(page_addr as *mut usize, HEAD.load(Ordering::Relaxed));
        }

        HEAD.store(page_addr, Ordering::Relaxed);
    }

    unlock();
}

pub fn frame_alloc() -> Option<FrameTracker> {
    lock();

    let head = HEAD.load(Ordering::Relaxed);
    if head == 0 {
        unlock();
        return None;
    }

    let next = unsafe { read_volatile(head as *const usize) };
    HEAD.store(next, Ordering::Relaxed);
    unlock();

    unsafe {
        write_bytes(head as *mut u8, 0, PAGE_SIZE);
    }

    Some(FrameTracker {
        ppn: PhysPageNum(head >> 12),
    })
}

pub fn free_frame_count() -> usize {
    let start = START_PPN.load(Ordering::Acquire);
    let end = END_PPN.load(Ordering::Acquire);

    lock();

    let mut current = HEAD.load(Ordering::Relaxed);
    let mut count = 0;

    while current != 0 {
        if current & (PAGE_SIZE - 1) != 0
            || current >> 12 < start
            || current >> 12 >= end
            || count >= total_frames()
        {
            unlock();
            panic!("free-frame list is corrupted");
        }

        current = unsafe { read_volatile(current as *const usize) };
        count += 1;
    }

    unlock();
    count
}

pub fn is_managed(ppn: PhysPageNum) -> bool {
    let start = START_PPN.load(Ordering::Acquire);
    let end = END_PPN.load(Ordering::Acquire);
    start <= ppn.0 && ppn.0 < end
}
