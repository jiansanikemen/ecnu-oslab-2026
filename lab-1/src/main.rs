#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

mod arch;
#[macro_use]
mod console;
mod sync;
mod uart;

static STARTED: AtomicBool = AtomicBool::new(false);

core::arch::global_asm!(include_str!("entry.S"));

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

        // 从 M-mode 切换到 S-mode，并跳转到 rust_main
        core::arch::asm!("mret", options(noreturn));
    }
}

#[no_mangle]
extern "C" fn rust_main() -> ! {
    let hart_id = arch::hart_id();

    if hart_id == 0 {
        uart::init();

        println!("ECNU OSLab rCore kernel entered S-mode");

        let selftest = sync::interrupt_nesting_selftest();
        println!(
            "interrupt nesting self-test: {}",
            if selftest { "PASS" } else { "FAIL" }
        );

        println!(
            "Rust formatting: char={} string={} d={} p={:x} x={:#x}",
            'A', "uart", -2025, 0x2025_u32, 0x1234_5678_8000_0000_u64,
        );

        // 发布 UART 已初始化完成
        STARTED.store(true, Ordering::Release);
    } else {
        // 等待 hart 0 完成 UART 初始化
        while !STARTED.load(Ordering::Acquire) {
            core::hint::spin_loop();
        }
    }

    let character = if hart_id == 0 { 'A' } else { 'B' };

    println!(
        "hart {} says: char={} string={} d={} p={:x} x={:#x}",
        hart_id, character, "uart", -2025, 0x2025_u32, 0x1234_5678_8000_0000_u64,
    );

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
