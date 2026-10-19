use super::{frame_alloc, FrameTracker, PhysPageNum, VirtPageNum};

#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(transparent)]
pub struct PageTableEntry(pub usize);

#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct PteFlags(pub usize);

impl PteFlags {
    pub const V: Self = Self(1 << 0);
    pub const R: Self = Self(1 << 1);
    pub const W: Self = Self(1 << 2);
    pub const X: Self = Self(1 << 3);
    pub const U: Self = Self(1 << 4);

    pub const fn bits(self) -> usize {
        self.0
    }

    pub fn contains(self, other: Self) -> bool {
        self.bits() & other.bits() == other.bits()
    }
}

impl core::ops::BitOr for PteFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl PageTableEntry {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub fn new(ppn: PhysPageNum, flags: PteFlags) -> Self {
        Self((ppn.0 << 10) | flags.bits())
    }

    pub fn ppn(self) -> PhysPageNum {
        PhysPageNum(self.0 >> 10)
    }

    pub fn flags(self) -> PteFlags {
        PteFlags(self.0 & 0x3ff)
    }

    pub fn is_valid(self) -> bool {
        self.flags().contains(PteFlags::V)
    }

    pub fn is_leaf(self) -> bool {
        self.flags().contains(PteFlags::R)
            || self.flags().contains(PteFlags::W)
            || self.flags().contains(PteFlags::X)
    }

    pub fn is_writable_without_readable(self) -> bool {
        self.flags().contains(PteFlags::W) && !self.flags().contains(PteFlags::R)
    }
}

fn pte_array(ppn: PhysPageNum) -> &'static mut [PageTableEntry; 512] {
    unsafe { &mut *(ppn.addr().0 as *mut [PageTableEntry; 512]) }
}

const MAX_PAGE_TABLE_FRAMES: usize = 128;

pub struct PageTable {
    root_ppn: PhysPageNum,
    frames: [Option<FrameTracker>; MAX_PAGE_TABLE_FRAMES],
    frame_count: usize,
}

impl PageTable {
    pub fn new() -> Option<Self> {
        let root = frame_alloc()?;
        let root_ppn = root.ppn();

        let mut frames = core::array::from_fn(|_| None);
        frames[0] = Some(root);

        Some(Self {
            root_ppn,
            frames,
            frame_count: 1,
        })
    }

    pub fn root_ppn(&self) -> PhysPageNum {
        self.root_ppn
    }

    fn rollback(
        &mut self,
        created: &[(PhysPageNum, usize); 2],
        created_len: usize,
        old_count: usize,
    ) {
        for index in (0..created_len).rev() {
            let (parent_ppn, pte_index) = created[index];
            pte_array(parent_ppn)[pte_index] = PageTableEntry::empty();
        }

        for index in old_count..self.frame_count {
            self.frames[index].take();
        }

        self.frame_count = old_count;
    }

    fn find_pte_create(&mut self, vpn: VirtPageNum) -> Result<&mut PageTableEntry, &'static str> {
        let indexes = vpn.indexes();
        let old_count = self.frame_count;
        let mut created = [(PhysPageNum(0), 0usize); 2];
        let mut created_len = 0;
        let mut current = self.root_ppn;

        for &index in &indexes[..2] {
            let pte_ptr = &mut pte_array(current)[index] as *mut PageTableEntry;
            let pte = unsafe { &mut *pte_ptr };

            if pte.is_valid() {
                if pte.is_leaf() {
                    self.rollback(&created, created_len, old_count);
                    return Err("unexpected huge-page leaf");
                }

                current = pte.ppn();
            } else {
                if self.frame_count >= MAX_PAGE_TABLE_FRAMES {
                    self.rollback(&created, created_len, old_count);
                    return Err("page-table frame capacity exhausted");
                }

                let frame = match frame_alloc() {
                    Some(frame) => frame,
                    None => {
                        self.rollback(&created, created_len, old_count);
                        return Err("out of physical frames");
                    }
                };

                let new_ppn = frame.ppn();

                self.frames[self.frame_count] = Some(frame);
                self.frame_count += 1;

                unsafe {
                    *pte_ptr = PageTableEntry::new(new_ppn, PteFlags::V);
                }

                created[created_len] = (current, index);
                created_len += 1;
                current = new_ppn;
            }
        }

        let leaf_ptr = &mut pte_array(current)[indexes[2]] as *mut PageTableEntry;

        Ok(unsafe { &mut *leaf_ptr })
    }

    fn find_pte(&self, vpn: VirtPageNum) -> Option<&PageTableEntry> {
        let indexes = vpn.indexes();
        let mut current = self.root_ppn;

        for &index in &indexes[..2] {
            let pte_ptr = &pte_array(current)[index] as *const PageTableEntry;
            let pte = unsafe { &*pte_ptr };

            if !pte.is_valid() || pte.is_leaf() {
                return None;
            }

            current = pte.ppn();
        }

        let leaf_ptr = &pte_array(current)[indexes[2]] as *const PageTableEntry;

        Some(unsafe { &*leaf_ptr })
    }

    fn find_pte_mut(&mut self, vpn: VirtPageNum) -> Option<&mut PageTableEntry> {
        let indexes = vpn.indexes();
        let mut current = self.root_ppn;

        for &index in &indexes[..2] {
            let pte_ptr = &mut pte_array(current)[index] as *mut PageTableEntry;
            let pte = unsafe { &mut *pte_ptr };

            if !pte.is_valid() || pte.is_leaf() {
                return None;
            }

            current = pte.ppn();
        }

        let leaf_ptr = &mut pte_array(current)[indexes[2]] as *mut PageTableEntry;

        Some(unsafe { &mut *leaf_ptr })
    }

    pub fn map(
        &mut self,
        vpn: VirtPageNum,
        ppn: PhysPageNum,
        flags: PteFlags,
    ) -> Result<(), &'static str> {
        if flags.contains(PteFlags::W) && !flags.contains(PteFlags::R) {
            return Err("writable page must also be readable");
        }

        if !flags.contains(PteFlags::R)
            && !flags.contains(PteFlags::W)
            && !flags.contains(PteFlags::X)
        {
            return Err("leaf page must have R, W or X permission");
        }

        let pte = self.find_pte_create(vpn)?;

        if pte.is_valid() {
            return Err("virtual page is already mapped");
        }

        *pte = PageTableEntry::new(ppn, flags | PteFlags::V);
        Ok(())
    }

    pub fn unmap(&mut self, vpn: VirtPageNum) -> Result<(), &'static str> {
        let pte = self.find_pte_mut(vpn).ok_or("virtual page is not mapped")?;

        if !pte.is_valid() || !pte.is_leaf() {
            return Err("virtual page is not mapped");
        }

        *pte = PageTableEntry::empty();
        Ok(())
    }

    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        let pte = self.find_pte(vpn)?;

        if pte.is_valid() && pte.is_leaf() {
            Some(*pte)
        } else {
            None
        }
    }
}
