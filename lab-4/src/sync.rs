use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::arch;

static INTERRUPT_DEPTH: [AtomicUsize; 2] = [const { AtomicUsize::new(0) }; 2];

static INTERRUPT_WAS_ENABLED: [AtomicBool; 2] = [const { AtomicBool::new(false) }; 2];

pub struct SpinLock {
    locked: AtomicBool,
    owner: AtomicUsize,
}

impl SpinLock {
    pub const fn new() -> Self {
        Self {
            locked: AtomicBool::new(false),
            owner: AtomicUsize::new(usize::MAX),
        }
    }

    pub fn lock(&self) -> SpinLockGuard<'_> {
        push_off();

        while self.locked.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }

        let hart_id = arch::hart_id();
        self.owner.store(hart_id, Ordering::Relaxed);

        SpinLockGuard { lock: self }
    }

    fn unlock(&self) {
        let hart_id = arch::hart_id();

        if self.owner.load(Ordering::Relaxed) != hart_id {
            panic!("spin lock owner mismatch");
        }

        self.owner.store(usize::MAX, Ordering::Relaxed);
        self.locked.store(false, Ordering::Release);

        pop_off();
    }
}

pub struct SpinLockGuard<'a> {
    lock: &'a SpinLock,
}

impl Drop for SpinLockGuard<'_> {
    fn drop(&mut self) {
        self.lock.unlock();
    }
}

fn push_off() {
    let hart_id = arch::hart_id();
    let depth = INTERRUPT_DEPTH[hart_id].fetch_add(1, Ordering::Relaxed);

    if depth == 0 {
        INTERRUPT_WAS_ENABLED[hart_id].store(arch::interrupts_enabled(), Ordering::Relaxed);
    }

    arch::disable_interrupts();
}

fn pop_off() {
    let hart_id = arch::hart_id();
    let depth = INTERRUPT_DEPTH[hart_id].load(Ordering::Relaxed);

    if depth == 0 {
        panic!("pop_off without push_off");
    }

    let remaining = INTERRUPT_DEPTH[hart_id].fetch_sub(1, Ordering::Relaxed) - 1;

    if remaining == 0 && INTERRUPT_WAS_ENABLED[hart_id].load(Ordering::Relaxed) {
        arch::enable_interrupts();
    }
}

pub fn interrupt_nesting_selftest() -> bool {
    let hart_id = arch::hart_id();
    let initial_enabled = arch::interrupts_enabled();

    push_off();

    let first_ok =
        !arch::interrupts_enabled() && INTERRUPT_DEPTH[hart_id].load(Ordering::Relaxed) == 1;

    push_off();

    let nested_ok =
        !arch::interrupts_enabled() && INTERRUPT_DEPTH[hart_id].load(Ordering::Relaxed) == 2;

    pop_off();

    let inner_exit_ok =
        !arch::interrupts_enabled() && INTERRUPT_DEPTH[hart_id].load(Ordering::Relaxed) == 1;

    pop_off();

    let restored_ok = arch::interrupts_enabled() == initial_enabled
        && INTERRUPT_DEPTH[hart_id].load(Ordering::Relaxed) == 0;

    first_ok && nested_ok && inner_exit_ok && restored_ok
}
