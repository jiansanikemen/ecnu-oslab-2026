#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

mod arch;
#[macro_use]
mod console;
mod mm;
mod plic;
mod sync;
mod timer;
mod trap;
mod uart;

static STARTED: AtomicBool = AtomicBool::new(false);
static TRAPS_READY: AtomicBool = AtomicBool::new(false);

core::arch::global_asm!(include_str!("entry.S"));
core::arch::global_asm!(include_str!("trap.S"));

unsafe extern "C" {
    fn ekernel();
}

#[no_mangle]
extern "C" fn start(hart_id: usize) -> ! {
    let mut mstatus = arch::read_mstatus();

    // 清除原来的 MPP 位
    mstatus &= !arch::MSTATUS_MPP_MASK;

    // 设置 mret 后进入 S-mode
    mstatus |= arch::MSTATUS_MPP_S;

    unsafe {
        // 设置 mret 返回后的状态和地址
        arch::write_mstatus(mstatus);
        arch::write_mepc(rust_main as *const () as usize);

        // 关闭分页
        arch::write_satp(0);

        // 允许 S-mode 访问实验所需的物理地址
        arch::write_pmpaddr0(usize::MAX);
        arch::write_pmpcfg0(arch::PMP_R | arch::PMP_W | arch::PMP_X | arch::PMP_A_TOR);

        // 保存当前 hart ID
        arch::write_tp(hart_id);

        arch::write_medeleg(0xffff);
        arch::write_mideleg((1usize << 1) | (1usize << 5) | (1usize << 9));

        timer::init_machine(hart_id);

        // 从 M-mode 切换到 S-mode，并跳转到 rust_main
        core::arch::asm!("mret", options(noreturn));
    }
}

#[no_mangle]
extern "C" fn rust_main() -> ! {
    let hart_id = arch::hart_id();

    if hart_id == 0 {
        uart::init();

        STARTED.store(true, Ordering::Release);

        unsafe {
            mm::frame_allocator_init(
                mm::PhysAddr(ekernel as *const () as usize),
                mm::PhysAddr(0x8800_0000),
            );
        }

        if !mm::run_tests() {
            loop {
                arch::wait_for_interrupt();
            }
        }
    } else {
        while !STARTED.load(Ordering::Acquire) {
            core::hint::spin_loop();
        }
    }

    let token = loop {
        let token = mm::kernel_token();

        if token != 0 {
            break token;
        }

        core::hint::spin_loop();
    };

    unsafe {
        arch::activate_page_table(token);
    }

    println!("hart {} paging enabled", hart_id);

    // 全局 PLIC 配置只由 hart 0 初始化一次。
    if hart_id == 0 {
        trap::init_global();

        TRAPS_READY.store(true, Ordering::Release);
    } else {
        while !TRAPS_READY.load(Ordering::Acquire) {
            core::hint::spin_loop();
        }
    }

    // 每个 hart 分别设置 stvec 和自己的中断使能位。
    trap::init_hart();

    // Trap 入口和 PLIC 准备完成后，再允许 UART 接收中断。
    if hart_id == 0 {
        uart::enable_receive_interrupt();
    }

    println!("hart {} traps enabled", hart_id);

    if hart_id == 0 {
        if trap::context_layout_ok() {
            println!("lab-3 context test: PASS");
        } else {
            println!("lab-3 context test: FAIL");

            loop {
                arch::wait_for_interrupt();
            }
        }

        while timer::ticks() < 3 {
            arch::wait_for_interrupt();
        }

        println!("lab-3 timer test: PASS ticks={}", timer::ticks());
    }

    loop {
        arch::wait_for_interrupt();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        arch::wait_for_interrupt();
    }
}
