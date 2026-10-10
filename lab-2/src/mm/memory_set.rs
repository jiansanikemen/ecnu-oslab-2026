use core::sync::atomic::{AtomicUsize, Ordering};

use super::{PageTable, PhysPageNum, PteFlags, VirtAddr, VirtPageNum, PAGE_SIZE};

const MEMORY_END: usize = 0x8800_0000;
const UART_BASE: usize = 0x1000_0000;

static KERNEL_TOKEN: AtomicUsize = AtomicUsize::new(0);

extern "C" {
    fn stext();
    fn etext();
    fn srodata();
    fn erodata();
    fn sdata();
    fn edata();
    fn sbss();
    fn ebss();
    fn ekernel();
}

fn map_identity_range(
    page_table: &mut PageTable,
    start: usize,
    end: usize,
    flags: PteFlags,
) -> Result<(), &'static str> {
    if start > end || start % PAGE_SIZE != 0 || end % PAGE_SIZE != 0 {
        return Err("mapping range is not page aligned");
    }

    let mut vpn = VirtAddr(start).floor();
    let end_vpn = VirtAddr(end).floor();

    while vpn.0 < end_vpn.0 {
        page_table.map(vpn, PhysPageNum(vpn.0), flags)?;

        vpn = VirtPageNum(vpn.0 + 1);
    }

    Ok(())
}

fn check_range(page_table: &PageTable, start: usize, end: usize, expected: PteFlags) -> bool {
    let mut vpn = VirtAddr(start).floor();
    let end_vpn = VirtAddr(end).floor();

    while vpn.0 < end_vpn.0 {
        let Some(pte) = page_table.translate(vpn) else {
            return false;
        };

        let flags = pte.flags();

        if !flags.contains(PteFlags::V) || !flags.contains(expected) || flags.contains(PteFlags::U)
        {
            return false;
        }

        if flags.contains(PteFlags::W) && flags.contains(PteFlags::X) {
            return false;
        }

        if !expected.contains(PteFlags::W) && flags.contains(PteFlags::W) {
            return false;
        }

        if !expected.contains(PteFlags::X) && flags.contains(PteFlags::X) {
            return false;
        }

        vpn = VirtPageNum(vpn.0 + 1);
    }

    true
}

pub fn kernel_space_init() -> Result<(), &'static str> {
    if KERNEL_TOKEN.load(Ordering::Acquire) != 0 {
        return Err("kernel page table already initialized");
    }

    let stext_addr = stext as *const () as usize;
    let etext_addr = etext as *const () as usize;
    let srodata_addr = srodata as *const () as usize;
    let erodata_addr = erodata as *const () as usize;
    let sdata_addr = sdata as *const () as usize;
    let edata_addr = edata as *const () as usize;
    let sbss_addr = sbss as *const () as usize;
    let ebss_addr = ebss as *const () as usize;
    let ekernel_addr = ekernel as *const () as usize;

    let boundaries = [
        stext_addr,
        etext_addr,
        srodata_addr,
        erodata_addr,
        sdata_addr,
        edata_addr,
        sbss_addr,
        ebss_addr,
        ekernel_addr,
    ];

    if boundaries.iter().any(|address| address % PAGE_SIZE != 0) {
        return Err("kernel boundary is not page aligned");
    }

    if !(stext_addr <= etext_addr
        && etext_addr <= srodata_addr
        && srodata_addr <= erodata_addr
        && erodata_addr <= sdata_addr
        && sdata_addr <= edata_addr
        && edata_addr <= sbss_addr
        && sbss_addr <= ebss_addr
        && ebss_addr <= ekernel_addr
        && ekernel_addr < MEMORY_END)
    {
        return Err("invalid kernel boundary order");
    }

    let mut page_table = PageTable::new().ok_or("cannot allocate root page table")?;

    let text_flags = PteFlags::R | PteFlags::X;
    let rodata_flags = PteFlags::R;
    let writable_flags = PteFlags::R | PteFlags::W;

    map_identity_range(&mut page_table, stext_addr, etext_addr, text_flags)?;

    map_identity_range(&mut page_table, srodata_addr, erodata_addr, rodata_flags)?;

    map_identity_range(&mut page_table, sdata_addr, edata_addr, writable_flags)?;

    map_identity_range(&mut page_table, sbss_addr, ebss_addr, writable_flags)?;

    map_identity_range(&mut page_table, ebss_addr, ekernel_addr, writable_flags)?;

    map_identity_range(&mut page_table, ekernel_addr, MEMORY_END, writable_flags)?;

    map_identity_range(
        &mut page_table,
        UART_BASE,
        UART_BASE + PAGE_SIZE,
        writable_flags,
    )?;

    if !check_range(&page_table, stext_addr, etext_addr, text_flags) {
        return Err("text permission check failed");
    }

    if !check_range(&page_table, srodata_addr, erodata_addr, rodata_flags) {
        return Err("rodata permission check failed");
    }

    if !check_range(&page_table, sdata_addr, MEMORY_END, writable_flags) {
        return Err("writable range permission check failed");
    }

    if !check_range(
        &page_table,
        UART_BASE,
        UART_BASE + PAGE_SIZE,
        writable_flags,
    ) {
        return Err("UART permission check failed");
    }

    let token = (8usize << 60) | page_table.root_ppn().0;

    KERNEL_TOKEN.store(token, Ordering::Release);

    core::mem::forget(page_table);

    Ok(())
}

pub fn kernel_token() -> usize {
    KERNEL_TOKEN.load(Ordering::Acquire)
}
