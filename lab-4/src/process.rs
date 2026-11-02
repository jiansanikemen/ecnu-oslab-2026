use crate::mm;
use core::sync::atomic::{AtomicBool, Ordering};

pub const USER_ENTRY: usize = 0x1000;
pub const TRAMPOLINE: usize = 0x7fff_f000;
pub const TRAP_CONTEXT: usize = TRAMPOLINE - mm::PAGE_SIZE;

static CURRENT_READY: AtomicBool = AtomicBool::new(false);
static mut CURRENT_PROCESS: Option<UserProcess> = None;

unsafe extern "C" {
    fn __switch(
        current: *mut crate::task_context::TaskContext,
        next: *const crate::task_context::TaskContext,
    );
}

/// # Safety
/// 只能由 hart 0 在关闭 S-mode 中断时调用一次。
/// 安装后不再移动或销毁当前进程。
#[inline(never)]
pub unsafe fn install_current_process(process: UserProcess) -> Result<(), &'static str> {
    if crate::arch::hart_id() != 0 {
        return Err("only hart 0 may install the first process");
    }

    if CURRENT_READY.load(Ordering::Acquire) {
        return Err("current process already installed");
    }

    unsafe {
        core::ptr::addr_of_mut!(CURRENT_PROCESS).write(Some(process));
    }

    CURRENT_READY.store(true, Ordering::Release);

    Ok(())
}

fn current_process() -> &'static UserProcess {
    assert!(
        crate::arch::hart_id() == 0,
        "current process is only available on hart 0"
    );

    assert!(
        CURRENT_READY.load(Ordering::Acquire),
        "current process is not installed"
    );

    unsafe {
        let slot = &*core::ptr::addr_of!(CURRENT_PROCESS);

        slot.as_ref().expect("missing current process")
    }
}

pub fn current_user_token() -> usize {
    current_process().token()
}

pub fn current_trap_context_ptr() -> *mut crate::trap_user::UserTrapContext {
    current_process().trap_context_ptr()
}

unsafe extern "C" {
    fn user_vector();
}

pub fn trampoline_ppn() -> mm::PhysPageNum {
    let address = user_vector as *const () as usize;

    mm::PhysPageNum(address / mm::PAGE_SIZE)
}

fn read_bytes<const N: usize>(data: &[u8], offset: usize) -> Result<[u8; N], &'static str> {
    let end = offset.checked_add(N).ok_or("ELF field offset overflow")?;

    let bytes = data.get(offset..end).ok_or("ELF field out of bounds")?;

    let mut result = [0u8; N];
    result.copy_from_slice(bytes);

    Ok(result)
}

fn read16(data: &[u8], offset: usize) -> Result<u16, &'static str> {
    Ok(u16::from_le_bytes(read_bytes::<2>(data, offset)?))
}

fn read32(data: &[u8], offset: usize) -> Result<u32, &'static str> {
    Ok(u32::from_le_bytes(read_bytes::<4>(data, offset)?))
}

fn read64(data: &[u8], offset: usize) -> Result<u64, &'static str> {
    Ok(u64::from_le_bytes(read_bytes::<8>(data, offset)?))
}

fn to_usize(value: u64) -> Result<usize, &'static str> {
    usize::try_from(value).map_err(|_| "ELF value does not fit usize")
}

struct ElfHeader {
    entry: usize,
    phoff: usize,
    phnum: usize,
    phentsize: usize,
}

struct LoadSegment {
    offset: usize,
    vaddr: usize,
    filesz: usize,
    memsz: usize,
    flags: u32,
}

struct ElfLayout {
    entry: usize,
    load_count: usize,
    heap_top: usize,
}

const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;

const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

fn parse_elf_header(data: &[u8]) -> Result<ElfHeader, &'static str> {
    if data.len() < 64 {
        return Err("ELF header is truncated");
    }

    if data.get(0..4) != Some(&b"\x7fELF"[..]) {
        return Err("invalid ELF magic");
    }

    // EI_CLASS：2 表示 ELF64。
    if data[4] != 2 {
        return Err("ELF is not 64-bit");
    }

    // EI_DATA：1 表示小端。
    if data[5] != 1 {
        return Err("ELF is not little-endian");
    }

    // EI_VERSION 和 e_version 均应为 1。
    if data[6] != 1 || read32(data, 20)? != 1 {
        return Err("unsupported ELF version");
    }

    // e_type：2 表示可执行文件。
    if read16(data, 16)? != 2 {
        return Err("ELF is not an executable");
    }

    // e_machine：243 表示 RISC-V。
    if read16(data, 18)? != 243 {
        return Err("ELF is not RISC-V");
    }

    if read16(data, 52)? != 64 {
        return Err("invalid ELF header size");
    }

    let entry = to_usize(read64(data, 24)?)?;
    let phoff = to_usize(read64(data, 32)?)?;
    let phentsize = usize::from(read16(data, 54)?);
    let phnum = usize::from(read16(data, 56)?);

    // 本实验支持标准 ELF64 程序头。
    if phentsize != 56 {
        return Err("unsupported program header size");
    }

    if phnum == 0 {
        return Err("ELF has no program headers");
    }

    if phoff < 64 {
        return Err("program header table overlaps ELF header");
    }

    let table_size = phentsize
        .checked_mul(phnum)
        .ok_or("program header table size overflow")?;

    let table_end = phoff
        .checked_add(table_size)
        .ok_or("program header table offset overflow")?;

    if table_end > data.len() {
        return Err("program header table out of bounds");
    }

    Ok(ElfHeader {
        entry,
        phoff,
        phnum,
        phentsize,
    })
}

fn parse_load_segment(
    data: &[u8],
    header: &ElfHeader,
    index: usize,
) -> Result<Option<LoadSegment>, &'static str> {
    if index >= header.phnum {
        return Err("program header index out of bounds");
    }

    let relative = index
        .checked_mul(header.phentsize)
        .ok_or("program header index overflow")?;

    let start = header
        .phoff
        .checked_add(relative)
        .ok_or("program header offset overflow")?;

    let end = start
        .checked_add(header.phentsize)
        .ok_or("program header end overflow")?;

    let ph = data.get(start..end).ok_or("program header out of bounds")?;

    let segment_type = read32(ph, 0)?;

    if segment_type == PT_DYNAMIC || segment_type == PT_INTERP {
        return Err("dynamic ELF is not supported");
    }

    if segment_type != PT_LOAD {
        return Ok(None);
    }

    let flags = read32(ph, 4)?;
    let offset = to_usize(read64(ph, 8)?)?;
    let vaddr = to_usize(read64(ph, 16)?)?;
    let filesz = to_usize(read64(ph, 32)?)?;
    let memsz = to_usize(read64(ph, 40)?)?;
    let align = to_usize(read64(ph, 48)?)?;

    if filesz > memsz {
        return Err("LOAD filesz exceeds memsz");
    }

    let file_end = offset
        .checked_add(filesz)
        .ok_or("LOAD file range overflow")?;

    if file_end > data.len() {
        return Err("LOAD file range out of bounds");
    }

    let memory_end = vaddr
        .checked_add(memsz)
        .ok_or("LOAD memory range overflow")?;

    if align > 1 && (!align.is_power_of_two() || vaddr % align != offset % align) {
        return Err("invalid LOAD alignment");
    }

    // 不需要为零长度段建立映射。
    if memsz == 0 {
        return Ok(None);
    }

    // 限定在本实验支持的低地址用户区域。
    // 保留零地址页，并避开固定的 trap context 和 trampoline。
    if vaddr < mm::PAGE_SIZE || memory_end > TRAP_CONTEXT {
        return Err("LOAD is outside user address range");
    }

    if flags & !(PF_R | PF_W | PF_X) != 0 {
        return Err("unsupported LOAD flags");
    }

    if flags & (PF_R | PF_W | PF_X) == 0 {
        return Err("LOAD has no access permissions");
    }

    if flags & PF_W != 0 && flags & PF_R == 0 {
        return Err("write-only LOAD is invalid");
    }

    if flags & PF_W != 0 && flags & PF_X != 0 {
        return Err("writable executable LOAD is forbidden");
    }

    Ok(Some(LoadSegment {
        offset,
        vaddr,
        filesz,
        memsz,
        flags,
    }))
}

fn validate_elf(data: &[u8], header: &ElfHeader) -> Result<ElfLayout, &'static str> {
    let mut load_count = 0;
    let mut heap_top = 0;
    let mut entry_is_executable = false;

    for index in 0..header.phnum {
        let Some(segment) = parse_load_segment(data, header, index)? else {
            continue;
        };

        let end = segment
            .vaddr
            .checked_add(segment.memsz)
            .ok_or("LOAD end overflow")?;

        let first_page = mm::VirtAddr(segment.vaddr).floor();
        let end_page = mm::VirtAddr(end).ceil();

        // 当前实验拒绝 LOAD 段在映射页范围上重叠，
        // 避免同一页被重复映射或赋予冲突权限。
        for previous in 0..index {
            let Some(other) = parse_load_segment(data, header, previous)? else {
                continue;
            };

            let other_end = other
                .vaddr
                .checked_add(other.memsz)
                .ok_or("LOAD end overflow")?;

            let other_first = mm::VirtAddr(other.vaddr).floor();
            let other_end_page = mm::VirtAddr(other_end).ceil();

            if first_page.0 < other_end_page.0 && other_first.0 < end_page.0 {
                return Err("LOAD page ranges overlap");
            }
        }

        if segment.flags & PF_X != 0 && header.entry >= segment.vaddr && header.entry < end {
            entry_is_executable = true;
        }

        load_count += 1;
        heap_top = heap_top.max(end);
    }

    if load_count == 0 {
        return Err("ELF has no nonempty LOAD segments");
    }

    if !entry_is_executable {
        return Err("ELF entry is not inside an executable LOAD");
    }

    Ok(ElfLayout {
        entry: header.entry,
        load_count,
        heap_top,
    })
}

// 本实验的固定小程序采用有上限的存储，避免引入动态堆。
const MAX_USER_PAGES: usize = 64;

struct OwnedUserPage {
    vpn: mm::VirtPageNum,
    frame: mm::FrameTracker,
    flags: mm::PteFlags,
}

struct UserPages {
    pages: [Option<OwnedUserPage>; MAX_USER_PAGES],
    count: usize,
}

impl UserPages {
    fn new() -> Self {
        Self {
            pages: core::array::from_fn(|_| None),
            count: 0,
        }
    }

    fn map_zeroed(
        &mut self,
        page_table: &mut mm::PageTable,
        vpn: mm::VirtPageNum,
        flags: mm::PteFlags,
    ) -> Result<mm::PhysPageNum, &'static str> {
        if self.count >= MAX_USER_PAGES {
            return Err("user page capacity exhausted");
        }

        let frame = mm::frame_alloc().ok_or("cannot allocate user frame")?;

        let ppn = frame.ppn();

        page_table.map(vpn, ppn, flags)?;

        self.pages[self.count] = Some(OwnedUserPage { vpn, frame, flags });
        self.count += 1;

        Ok(ppn)
    }
}

fn user_segment_flags(flags: u32) -> mm::PteFlags {
    let mut result = mm::PteFlags::U;

    if flags & PF_R != 0 {
        result = result | mm::PteFlags::R;
    }

    if flags & PF_W != 0 {
        result = result | mm::PteFlags::W;
    }

    if flags & PF_X != 0 {
        result = result | mm::PteFlags::X;
    }

    result
}

fn load_elf_segments(
    data: &[u8],
    header: &ElfHeader,
    page_table: &mut mm::PageTable,
    pages: &mut UserPages,
) -> Result<ElfLayout, &'static str> {
    // 先完整校验，再开始分配物理页。
    let layout = validate_elf(data, header)?;

    for index in 0..header.phnum {
        let Some(segment) = parse_load_segment(data, header, index)? else {
            continue;
        };

        let memory_end = segment
            .vaddr
            .checked_add(segment.memsz)
            .ok_or("LOAD memory end overflow")?;

        let file_virtual_end = segment
            .vaddr
            .checked_add(segment.filesz)
            .ok_or("LOAD file virtual end overflow")?;

        let first_page = mm::VirtAddr(segment.vaddr).floor();
        let end_page = mm::VirtAddr(memory_end).ceil();
        let flags = user_segment_flags(segment.flags);

        for page_number in first_page.0..end_page.0 {
            let vpn = mm::VirtPageNum(page_number);

            let ppn = pages.map_zeroed(page_table, vpn, flags)?;

            let page_start = page_number
                .checked_mul(mm::PAGE_SIZE)
                .ok_or("user page address overflow")?;

            let page_end = page_start
                .checked_add(mm::PAGE_SIZE)
                .ok_or("user page end overflow")?;

            // 当前页与段的文件数据所覆盖的虚拟地址范围求交集。
            let copy_start = page_start.max(segment.vaddr);
            let copy_end = page_end.min(file_virtual_end);

            if copy_start >= copy_end {
                continue;
            }

            let segment_offset = copy_start - segment.vaddr;
            let destination_offset = copy_start - page_start;
            let copy_len = copy_end - copy_start;

            let source_start = segment
                .offset
                .checked_add(segment_offset)
                .ok_or("ELF copy source overflow")?;

            let source_end = source_start
                .checked_add(copy_len)
                .ok_or("ELF copy source end overflow")?;

            let source = data
                .get(source_start..source_end)
                .ok_or("ELF copy source out of bounds")?;

            unsafe {
                let destination = (ppn.addr().0 as *mut u8).add(destination_offset);

                core::ptr::copy_nonoverlapping(source.as_ptr(), destination, copy_len);
            }
        }
    }

    Ok(layout)
}

pub struct UserProcess {
    pub pid: usize,
    pub entry: usize,
    pub heap_top: usize,
    pub user_stack_pages: usize,
    pub user_sp: usize,

    page_table: mm::PageTable,
    pages: UserPages,

    guard_start: usize,
    stack_bottom: usize,
    context_ppn: mm::PhysPageNum,

    kernel_stack: mm::FrameTracker,

    pub task_context: crate::task_context::TaskContext,
}

impl UserProcess {
    pub fn initialize_trap_context(&mut self) -> Result<(), &'static str> {
        if !crate::trap_user::layout_ok() {
            return Err("invalid UserTrapContext layout");
        }

        if self.user_sp % 16 != 0 || self.kernel_stack_top() % 16 != 0 {
            return Err("unaligned process stack");
        }

        let context = crate::trap_user::UserTrapContext::new(
            self.entry,
            self.user_sp,
            mm::kernel_token(),
            self.kernel_stack_top(),
            crate::trap_user::user_trap_handler as *const () as usize,
        );

        unsafe {
            self.trap_context_ptr().write(context);
        }

        let kernel_sp = self.kernel_stack_top();

        self.task_context.goto_trap_return(kernel_sp);

        Ok(())
    }
    pub fn from_elf(data: &[u8]) -> Result<Self, &'static str> {
        let header = parse_elf_header(data)?;

        // 先完成输入校验，避免为明显非法的文件分配内存。
        validate_elf(data, &header)?;

        let kernel_satp = mm::kernel_token();

        if kernel_satp == 0 {
            return Err("kernel page table is not initialized");
        }

        // 1. 独立用户页表。
        let mut page_table = mm::PageTable::new().ok_or("cannot allocate user root page table")?;

        // 2. 与内核页表共享同一跳板物理页。
        page_table.map(
            mm::VirtAddr(TRAMPOLINE).floor(),
            trampoline_ppn(),
            mm::PteFlags::R | mm::PteFlags::X,
        )?;

        // 保存本进程拥有的普通页面。
        let mut pages = UserPages::new();

        // 3. 装载 ELF 段。
        let layout = load_elf_segments(data, &header, &mut page_table, &mut pages)?;

        // 4. 最高段之后先保留一个 guard page，
        // 再映射一页用户栈。
        let guard_start = layout
            .heap_top
            .checked_add(mm::PAGE_SIZE - 1)
            .ok_or("guard address overflow")?
            & !(mm::PAGE_SIZE - 1);

        let stack_bottom = guard_start
            .checked_add(mm::PAGE_SIZE)
            .ok_or("user stack bottom overflow")?;

        let user_sp = stack_bottom
            .checked_add(mm::PAGE_SIZE)
            .ok_or("user stack top overflow")?;

        if user_sp > TRAP_CONTEXT {
            return Err("user stack overlaps trap context");
        }

        pages.map_zeroed(
            &mut page_table,
            mm::VirtAddr(stack_bottom).floor(),
            mm::PteFlags::R | mm::PteFlags::W | mm::PteFlags::U,
        )?;

        // guard_start 所在页故意不映射。

        // 5. TrapContext 页面，S-mode 可读写，U-mode 不可访问。
        let context_ppn = pages.map_zeroed(
            &mut page_table,
            mm::VirtAddr(TRAP_CONTEXT).floor(),
            mm::PteFlags::R | mm::PteFlags::W,
        )?;

        // 6. 独立内核栈；该物理页已在内核 RAM 恒等映射范围内。
        let kernel_stack = mm::frame_alloc().ok_or("cannot allocate process kernel stack")?;

        Ok(Self {
            pid: 0,
            entry: layout.entry,
            heap_top: layout.heap_top,
            user_stack_pages: 1,
            user_sp,

            page_table,
            pages,

            guard_start,
            stack_bottom,
            context_ppn,

            kernel_stack,

            // 任务五中再设置首次切换入口和栈指针。
            task_context: crate::task_context::TaskContext::zero(),
        })
    }
    pub fn token(&self) -> usize {
        (8usize << 60) | self.page_table.root_ppn().0
    }

    pub fn kernel_stack_top(&self) -> usize {
        self.kernel_stack.ppn().addr().0 + mm::PAGE_SIZE
    }

    pub fn trap_context_ptr(&self) -> *mut crate::trap_user::UserTrapContext {
        self.context_ppn.addr().0 as *mut crate::trap_user::UserTrapContext
    }
    fn check_mapping(
        &self,
        address: usize,
        expected_ppn: mm::PhysPageNum,
        expected_flags: mm::PteFlags,
    ) -> bool {
        let Some(pte) = self.page_table.translate(mm::VirtAddr(address).floor()) else {
            println!("missing user mapping: va={:#x}", address);
            return false;
        };

        // 检查 V/R/W/X/U；允许硬件维护 A/D 等其他状态位。
        let permission_mask = mm::PteFlags::V.bits()
            | mm::PteFlags::R.bits()
            | mm::PteFlags::W.bits()
            | mm::PteFlags::X.bits()
            | mm::PteFlags::U.bits();

        let expected = (expected_flags | mm::PteFlags::V).bits();

        if pte.ppn() != expected_ppn || pte.flags().bits() & permission_mask != expected {
            println!(
                "invalid user mapping: va={:#x} ppn={:#x} flags={:#x}",
                address,
                pte.ppn().0,
                pte.flags().bits(),
            );

            return false;
        }

        true
    }
    pub fn layout_check(&self) -> bool {
        // 用户栈必须紧邻 guard page，并满足 16 字节对齐。
        if self.pid != 0
            || self.user_stack_pages != 1
            || self.user_sp % 16 != 0
            || self.guard_start % mm::PAGE_SIZE != 0
            || self.heap_top > self.guard_start
            || self.guard_start.checked_add(mm::PAGE_SIZE) != Some(self.stack_bottom)
            || self.stack_bottom.checked_add(mm::PAGE_SIZE) != Some(self.user_sp)
            || self.user_sp > TRAP_CONTEXT
        {
            println!(
                "invalid user stack layout: guard={:#x} stack={:#x} sp={:#x}",
                self.guard_start, self.stack_bottom, self.user_sp,
            );

            return false;
        }

        // 逐页核对本进程拥有的映射。
        for index in 0..self.pages.count {
            let Some(page) = self.pages.pages[index].as_ref() else {
                println!("missing owned page record: index={}", index);
                return false;
            };

            let address = page.vpn.0 * mm::PAGE_SIZE;

            if !self.check_mapping(address, page.frame.ppn(), page.flags) {
                return false;
            }

            let flags = page.flags;

            // 用户 TrapContext 只能由 S-mode 访问。
            if address == TRAP_CONTEXT {
                if flags.bits() != (mm::PteFlags::R | mm::PteFlags::W).bits() {
                    println!("invalid context permissions: va={:#x}", address);
                    return false;
                }
            } else if address == self.stack_bottom {
                if flags.bits() != (mm::PteFlags::R | mm::PteFlags::W | mm::PteFlags::U).bits() {
                    println!("invalid stack permissions: va={:#x}", address);
                    return false;
                }
            } else {
                // 其他拥有的页应属于 ELF 段。
                if address >= self.guard_start
                    || !flags.contains(mm::PteFlags::U)
                    || (flags.contains(mm::PteFlags::W) && flags.contains(mm::PteFlags::X))
                {
                    println!("invalid ELF page: va={:#x}", address);
                    return false;
                }
            }
        }

        // 核对栈和上下文确实存在。
        if self
            .page_table
            .translate(mm::VirtAddr(self.stack_bottom).floor())
            .is_none()
        {
            println!("user stack is missing: va={:#x}", self.stack_bottom);
            return false;
        }

        if !self.check_mapping(
            TRAP_CONTEXT,
            self.context_ppn,
            mm::PteFlags::R | mm::PteFlags::W,
        ) {
            return false;
        }

        // 跳板共享代码物理页，但不能被用户直接访问。
        if !self.check_mapping(
            TRAMPOLINE,
            trampoline_ppn(),
            mm::PteFlags::R | mm::PteFlags::X,
        ) {
            return false;
        }

        // 本次固定镜像的入口页应可读、可执行、用户可访问。
        let Some(entry_pte) = self.page_table.translate(mm::VirtAddr(self.entry).floor()) else {
            println!("user entry is not mapped: va={:#x}", self.entry);
            return false;
        };

        let entry_flags = entry_pte.flags();

        if !entry_flags
            .contains(mm::PteFlags::V | mm::PteFlags::R | mm::PteFlags::X | mm::PteFlags::U)
            || entry_flags.contains(mm::PteFlags::W)
        {
            println!("invalid entry permissions: va={:#x}", self.entry);
            return false;
        }

        // 这些地址不应出现在用户地址空间。
        for address in [
            self.guard_start,
            0,           // 零地址页
            0x1000_0000, // UART
            0x0c00_0000, // PLIC priority
            0x0c00_2000, // PLIC enable
            0x0c20_1000, // PLIC threshold/claim
            0x8000_0000, // 内核入口
            self.kernel_stack.ppn().addr().0,
        ] {
            if self
                .page_table
                .translate(mm::VirtAddr(address).floor())
                .is_some()
            {
                println!("unexpected user mapping: va={:#x}", address);
                return false;
            }
        }

        true
    }
}

#[inline(never)]
pub fn address_space_self_test() -> bool {
    let before = mm::free_frame_count();

    let mut process = match UserProcess::from_elf(crate::user_image::USER_ELF) {
        Ok(process) => process,
        Err(error) => {
            println!("lab-4 process creation failed: {}", error);

            let after = mm::free_frame_count();

            if after != before {
                println!(
                    "frame leak after failure: before={} after={}",
                    before, after,
                );
            }

            return false;
        }
    };

    println!(
        "lab-4 ELF loaded: entry={:#x} heap_top={:#x}",
        process.entry, process.heap_top,
    );

    println!(
        "lab-4 user stack: bottom={:#x} top={:#x}",
        process.stack_bottom, process.user_sp,
    );

    let layout_ok = process.layout_check();

    let context_ok = match process.initialize_trap_context() {
        Ok(()) => {
            let context = unsafe { &*process.trap_context_ptr() };

            context.x[2] == process.user_sp
                && context.sepc == process.entry
                && context.kernel_satp == mm::kernel_token()
                && context.kernel_sp == process.kernel_stack_top()
                && context.trap_handler == crate::trap_user::user_trap_handler as *const () as usize
                && context.sstatus & (1usize << 8) == 0
                && context.sstatus & (1usize << 1) == 0
                && context.sstatus & (1usize << 5) != 0
        }
        Err(error) => {
            println!("context initialization failed: {}", error);
            false
        }
    };

    if context_ok {
        println!("lab-4 user context test: PASS");
    } else {
        println!("lab-4 user context test: FAIL");
    }

    // 此进程的用户页表尚未激活，可以直接销毁。
    drop(process);

    let after = mm::free_frame_count();
    let reclaimed = after == before;

    if !reclaimed {
        println!("frame reclaim failed: before={} after={}", before, after,);
    }

    if layout_ok {
        println!("lab-4 address-space test: PASS");
    } else {
        println!("lab-4 address-space test: FAIL");
    }

    if reclaimed {
        println!("lab-4 frame reclaim test: PASS");
    } else {
        println!("lab-4 frame reclaim test: FAIL");
    }

    layout_ok && context_ok && reclaimed
}

#[inline(never)]
pub fn run_first() -> ! {
    assert!(crate::arch::hart_id() == 0);

    // 创建、安装进程和首次切换期间保持 S-mode 中断关闭。
    crate::arch::disable_interrupts();

    let result = (|| -> Result<(), &'static str> {
        let mut process = UserProcess::from_elf(crate::user_image::USER_ELF)?;

        if !process.layout_check() {
            return Err("first process layout check failed");
        }

        println!("lab-4 address-space test: PASS");

        process.initialize_trap_context()?;

        unsafe {
            install_current_process(process)?;
        }

        Ok(())
    })();

    if let Err(error) = result {
        println!("cannot start first process: {}", error);

        loop {
            core::hint::spin_loop();
        }
    }

    println!("lab-4 entering user mode");

    let mut boot_context = crate::task_context::TaskContext::zero();

    let next_context = &current_process().task_context as *const crate::task_context::TaskContext;

    unsafe {
        __switch(&mut boot_context as *mut _, next_context);
    }

    // 本实验不会调度回启动执行流。
    println!("unexpected return from first process");

    loop {
        core::hint::spin_loop();
    }
}
