use core::sync::atomic::{AtomicBool, Ordering};

const UART0: usize = 0x1000_0000;

const THR: usize = 0;
const IER: usize = 1;
const FCR: usize = 2;
const LCR: usize = 3;
const LSR: usize = 5;

const LCR_BAUD_LATCH: u8 = 1 << 7;
const LCR_EIGHT_BITS: u8 = 3;
const FCR_FIFO_ENABLE: u8 = 1;
const FCR_FIFO_CLEAR: u8 = 3 << 1;
const LSR_TX_IDLE: u8 = 1 << 5;

const RHR: usize = 0;
const LSR_RX_READY: u8 = 1 << 0;
const IER_RX_ENABLE: u8 = 1 << 0;

static UART_TEST_REPORTED: AtomicBool = AtomicBool::new(false);

#[inline]
unsafe fn write_register(offset: usize, value: u8) {
    (UART0 as *mut u8).add(offset).write_volatile(value);
}

#[inline]
unsafe fn read_register(offset: usize) -> u8 {
    (UART0 as *const u8).add(offset).read_volatile()
}

pub fn init() {
    unsafe {
        // 关闭 UART 中断
        write_register(IER, 0);

        // 打开波特率锁存访问
        write_register(LCR, LCR_BAUD_LATCH);

        // 设置波特率
        write_register(0, 0x03);
        write_register(1, 0);

        // 设置为 8 位数据格式
        write_register(LCR, LCR_EIGHT_BITS);

        // 启用并清空 FIFO
        write_register(FCR, FCR_FIFO_ENABLE | FCR_FIFO_CLEAR);
    }
}

pub fn putchar(character: u8) {
    unsafe {
        // 等待 UART 发送寄存器空闲
        while read_register(LSR) & LSR_TX_IDLE == 0 {
            core::hint::spin_loop();
        }

        // 发送字符
        write_register(THR, character);
    }
}

pub fn enable_receive_interrupt() {
    unsafe {
        write_register(IER, IER_RX_ENABLE);
    }
}

pub fn getchar() -> Option<u8> {
    unsafe {
        if read_register(LSR) & LSR_RX_READY == 0 {
            None
        } else {
            Some(read_register(RHR))
        }
    }
}

pub fn handle_receive_interrupt() {
    while let Some(byte) = getchar() {
        match byte {
            8 | 127 => {
                putchar(8);
                putchar(b' ');
                putchar(8);
            }
            b'\r' | b'\n' => {
                putchar(b'\n');
            }
            byte => {
                putchar(byte);
            }
        }

        if byte == b'K' && !UART_TEST_REPORTED.swap(true, Ordering::AcqRel) {
            println!("\nlab-3 uart test: PASS byte=K");
        }
    }
}
