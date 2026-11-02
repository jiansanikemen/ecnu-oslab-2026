use crate::{arch, plic, timer, uart};

const SCAUSE_INTERRUPT: usize = 1usize << (usize::BITS - 1);
const S_SOFTWARE: usize = 1;
const S_EXTERNAL: usize = 9;

unsafe extern "C" {
    fn kernel_vector();
}

#[repr(C)]
pub struct KernelTrapContext {
    pub x: [usize; 32],
}

pub fn context_layout_ok() -> bool {
    core::mem::size_of::<KernelTrapContext>() == 32 * core::mem::size_of::<usize>()
}

pub fn init_global() {
    plic::init();
}

pub fn init_hart() {
    unsafe {
        arch::set_stvec(kernel_vector as *const () as usize);
    }

    plic::init_hart(arch::hart_id());

    arch::enable_supervisor_trap_interrupts();
}

#[no_mangle]
extern "C" fn kernel_trap_handler(_context: &mut KernelTrapContext) {
    let scause: usize;
    let sepc: usize;
    let stval: usize;

    unsafe {
        core::arch::asm!("csrr {}, scause", out(reg) scause);
        core::arch::asm!("csrr {}, sepc", out(reg) sepc);
        core::arch::asm!("csrr {}, stval", out(reg) stval);
    }

    let is_interrupt = scause & SCAUSE_INTERRUPT != 0;
    let cause = scause & !SCAUSE_INTERRUPT;

    if is_interrupt && cause == S_SOFTWARE {
        timer::update(arch::hart_id());
        return;
    }

    if is_interrupt && cause == S_EXTERNAL {
        let irq = plic::claim();

        if irq != 0 {
            if irq == plic::UART_IRQ {
                uart::handle_receive_interrupt();
            }

            plic::complete(irq);

            if irq != plic::UART_IRQ {
                println!("unknown external IRQ: {}", irq);
            }
        }

        return;
    }

    println!(
        "unhandled trap: hart={} scause={:#x} sepc={:#x} stval={:#x}",
        arch::hart_id(),
        scause,
        sepc,
        stval,
    );

    loop {
        core::hint::spin_loop();
    }
}
