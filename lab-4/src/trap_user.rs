use crate::arch;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const SYS_HELLOWORLD: usize = 1;

static HELLO_COUNT: AtomicUsize = AtomicUsize::new(0);
static USER_TICKS: AtomicUsize = AtomicUsize::new(0);
static USER_UART_REPORTED: AtomicBool = AtomicBool::new(false);

unsafe extern "C" {
    fn user_vector();
    fn user_restore();
    fn kernel_vector();
}

#[no_mangle]
pub extern "C" fn trap_return() -> ! {
    // 从这里开始到 sret，不允许 S-mode 中断打断返回过程。
    arch::disable_interrupts();

    let user_token = crate::process::current_user_token();

    let vector_address = user_vector as *const () as usize;
    let restore_address = user_restore as *const () as usize;

    let restore_offset = restore_address - vector_address;

    let restore_virtual = crate::process::TRAMPOLINE + restore_offset;

    init();

    unsafe {
        core::arch::asm!(
            "fence.i",
            "jr {restore}",
            restore = in(reg) restore_virtual,
            in("a0") crate::process::TRAP_CONTEXT,
            in("a1") user_token,
            options(noreturn),
        );
    }
}

#[repr(C)]
pub struct UserTrapContext {
    pub x: [usize; 32],
    pub sstatus: usize,
    pub sepc: usize,
    pub kernel_satp: usize,
    pub kernel_sp: usize,
    pub trap_handler: usize,
}

impl UserTrapContext {
    pub fn new(
        entry: usize,
        user_sp: usize,
        kernel_satp: usize,
        kernel_sp: usize,
        trap_handler: usize,
    ) -> Self {
        let mut sstatus: usize;

        unsafe {
            core::arch::asm!(
                "csrr {}, sstatus",
                out(reg) sstatus,
            );
        }

        // SPP=0：sret 返回 U-mode。
        sstatus &= !(1usize << 8);

        // 返回流程准备期间，不打开 S-mode 总中断。
        sstatus &= !(1usize << 1);

        // SPIE=1：sret 时恢复 S-mode 中断使能状态。
        sstatus |= 1usize << 5;

        // 不允许内核普通访存直接访问 U 页面。
        sstatus &= !(1usize << 18);

        let mut context = Self {
            x: [0; 32],
            sstatus,
            sepc: entry,
            kernel_satp,
            kernel_sp,
            trap_handler,
        };

        // x2 是用户栈指针，不能填入内核栈顶。
        context.x[2] = user_sp;

        context
    }
}

pub fn layout_ok() -> bool {
    core::mem::size_of::<usize>() == 8
        && core::mem::size_of::<UserTrapContext>() == 296
        && core::mem::offset_of!(UserTrapContext, x) == 0
        && core::mem::offset_of!(UserTrapContext, sstatus) == 256
        && core::mem::offset_of!(UserTrapContext, sepc) == 264
        && core::mem::offset_of!(UserTrapContext, kernel_satp) == 272
        && core::mem::offset_of!(UserTrapContext, kernel_sp) == 280
        && core::mem::offset_of!(UserTrapContext, trap_handler) == 288
}

pub fn init() {
    arch::disable_interrupts();

    unsafe {
        arch::set_stvec(crate::process::TRAMPOLINE);
    }
}

#[no_mangle]
pub extern "C" fn user_trap_handler() -> ! {
    // 此时已切回内核页表和内核栈，S-mode 中断关闭。
    // 内核执行期间发生异常，应进入内核 Trap 入口。
    unsafe {
        arch::set_stvec(kernel_vector as *const () as usize);
    }

    let scause: usize;
    let sepc: usize;
    let stval: usize;

    unsafe {
        core::arch::asm!("csrr {}, scause", out(reg) scause);
        core::arch::asm!("csrr {}, sepc", out(reg) sepc);
        core::arch::asm!("csrr {}, stval", out(reg) stval);
    }

    let interrupt_bit = 1usize << (usize::BITS - 1);
    let is_interrupt = scause & interrupt_bit != 0;
    let cause = scause & !interrupt_bit;

    if !is_interrupt && cause == 8 {
        // 异常码 8：来自 U-mode 的 ecall。
        let syscall_id;

        {
            let context = unsafe { &mut *crate::process::current_trap_context_ptr() };

            // ecall 固定为 4 字节，返回时跳过已处理的指令。
            context.sepc += 4;
            syscall_id = context.x[17]; // a7

            if syscall_id == SYS_HELLOWORLD {
                context.x[10] = 0; // a0：系统调用返回值
            }
        }

        if syscall_id == SYS_HELLOWORLD {
            println!("proczero: hello world!");

            let count = HELLO_COUNT.fetch_add(1, Ordering::AcqRel) + 1;

            if count == 2 {
                println!("lab-4 user syscall: PASS");
            }

            trap_return();
        }

        println!("unknown user syscall: {}", syscall_id);
    } else if is_interrupt && cause == 1 {
        // M-mode 时钟入口转交的软件中断。
        crate::timer::update(arch::hart_id());

        let count = USER_TICKS.fetch_add(1, Ordering::AcqRel) + 1;

        if count == 3 {
            println!("lab-4 user timer: PASS");
        }

        trap_return();
    } else if is_interrupt && cause == 9 {
        let irq = crate::plic::claim();

        if irq == crate::plic::UART_IRQ {
            crate::uart::handle_receive_interrupt();
        }

        if irq != 0 {
            crate::plic::complete(irq);
        }

        if irq != 0 && irq != crate::plic::UART_IRQ {
            println!("unknown user external IRQ: {}", irq);
        } else {
            if crate::uart::test_byte_received() && !USER_UART_REPORTED.swap(true, Ordering::AcqRel)
            {
                println!("lab-4 user uart: PASS");
            }

            trap_return();
        }
    }

    println!(
        "unhandled user trap: scause={:#x} sepc={:#x} stval={:#x}",
        scause, sepc, stval,
    );

    loop {
        core::hint::spin_loop();
    }
}
