use core::ptr::{read_volatile, write_volatile};
mod address;
mod frame_allocator;
mod memory_set;
mod page_table;

pub use address::{PhysAddr, PhysPageNum, VirtAddr, VirtPageNum, PAGE_SIZE};
pub use frame_allocator::{
    frame_alloc, frame_allocator_init, free_frame_count, is_managed, FrameTracker,
};
pub use memory_set::{kernel_space_init, kernel_token};
pub use page_table::{PageTable, PageTableEntry, PteFlags};

fn pass_text(passed: bool) -> &'static str {
    if passed {
        "PASS"
    } else {
        "FAIL"
    }
}

pub fn run_tests() -> bool {
    let address_ok = address_test();
    println!("lab-2 address test: {}", pass_text(address_ok));

    let frame_ok = frame_test();
    println!("lab-2 frame test: {}", pass_text(frame_ok));

    let page_table_ok = page_table_test();
    println!("lab-2 page-table test: {}", pass_text(page_table_ok));

    let prerequisites_ok = address_ok && frame_ok && page_table_ok;

    let kernel_ok = prerequisites_ok && kernel_space_init().is_ok();

    println!("lab-2 kernel-space test: {}", pass_text(kernel_ok));

    prerequisites_ok && kernel_ok
}

fn address_test() -> bool {
    let va = VirtAddr(0x12345);

    va.floor().0 == 0x12
        && va.ceil().0 == 0x13
        && va.page_offset() == 0x345
        && VirtPageNum(0x12345).indexes() == [0x0, 0x91, 0x145]
}

fn frame_test() -> bool {
    let before = free_frame_count();

    if before < 3 {
        return false;
    }

    let a = match frame_alloc() {
        Some(frame) => frame,
        None => return false,
    };

    let b = match frame_alloc() {
        Some(frame) => frame,
        None => return false,
    };

    if a.ppn() == b.ppn() || !is_managed(a.ppn()) || !is_managed(b.ppn()) {
        return false;
    }

    unsafe {
        write_volatile(a.ppn().addr().0 as *mut u8, 0x5a);
    }

    let a_ppn = a.ppn();
    drop(a);

    let c = match frame_alloc() {
        Some(frame) => frame,
        None => return false,
    };

    let reused = c.ppn() == a_ppn;

    let cleared = unsafe { read_volatile(c.ppn().addr().0 as *const u8) == 0 };

    drop(b);
    drop(c);

    reused && cleared && free_frame_count() == before
}

fn page_table_test() -> bool {
    let before = free_frame_count();

    let result = (|| -> Option<()> {
        let mut page_table = PageTable::new()?;
        let target = frame_alloc()?;
        let other = frame_alloc()?;

        let vpn = VirtPageNum(0x12345);

        // 初始状态：未映射
        if page_table.translate(vpn).is_some() {
            return None;
        }

        // 正常映射
        page_table
            .map(vpn, target.ppn(), PteFlags::R | PteFlags::W)
            .ok()?;

        let saved = page_table.translate(vpn)?;

        if saved.ppn() != target.ppn() {
            return None;
        }

        // 重复映射必须失败，且不能覆盖原映射
        if page_table
            .map(vpn, other.ppn(), PteFlags::R | PteFlags::W)
            .is_ok()
        {
            return None;
        }

        if page_table.translate(vpn)? != saved {
            return None;
        }

        // W-only 权限非法
        if page_table.map(vpn, other.ppn(), PteFlags::W).is_ok() {
            return None;
        }

        // 没有 R/W/X 的权限非法
        if page_table.map(vpn, other.ppn(), PteFlags::V).is_ok() {
            return None;
        }

        // 解除映射
        page_table.unmap(vpn).ok()?;

        if page_table.translate(vpn).is_some() {
            return None;
        }

        // 重复解除映射必须失败
        if page_table.unmap(vpn).is_ok() {
            return None;
        }

        Some(())
    })();

    result.is_some() && free_frame_count() == before
}
