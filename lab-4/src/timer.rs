use core::sync::atomic::{AtomicUsize, Ordering};

pub const INTERVAL: usize = 1_000_000;

const CLINT_MTIME: usize = 0x0200_bff8;
const CLINT_MTIMECMP: usize = 0x0200_4000;
const MAX_HARTS: usize = 2;

static TICKS: AtomicUsize = AtomicUsize::new(0);

static mut TIMER_SCRATCH: [[usize; 5]; MAX_HARTS] = [[0; 5]; MAX_HARTS];

unsafe extern "C" {
    fn timer_vector();
}

/// # Safety
/// 只能由对应 hart 在 M-mode 启动阶段调用一次。
pub unsafe fn init_machine(hart_id: usize) {
    assert!(hart_id < MAX_HARTS);

    let mtimecmp = (CLINT_MTIMECMP + hart_id * 8) as *mut usize;

    let now = unsafe { core::ptr::read_volatile(CLINT_MTIME as *const usize) };

    unsafe {
        core::ptr::write_volatile(mtimecmp, now.wrapping_add(INTERVAL));
    }

    let base = core::ptr::addr_of_mut!(TIMER_SCRATCH).cast::<usize>();

    let scratch = unsafe { base.add(hart_id * 5) };

    unsafe {
        scratch.add(3).write(mtimecmp as usize);
        scratch.add(4).write(INTERVAL);

        core::arch::asm!(
            "csrw mscratch, {}",
            in(reg) scratch
        );

        core::arch::asm!(
            "csrw mtvec, {}",
            in(reg) timer_vector as *const () as usize
        );

        core::arch::asm!(
            "csrs mstatus, {}",
            in(reg) (1usize << 3)
        );

        core::arch::asm!(
            "csrs mie, {}",
            in(reg) (1usize << 7)
        );
    }
}

pub fn update(hart_id: usize) {
    if hart_id == 0 {
        TICKS.fetch_add(1, Ordering::AcqRel);
    }

    crate::arch::clear_supervisor_software_interrupt();
}

pub fn ticks() -> usize {
    TICKS.load(Ordering::Acquire)
}
