#[repr(C)]
pub struct TaskContext {
    pub ra: usize,
    pub sp: usize,
    pub s: [usize; 12],
}

impl TaskContext {
    pub const fn zero() -> Self {
        Self {
            ra: 0,
            sp: 0,
            s: [0; 12],
        }
    }

    pub fn goto_trap_return(&mut self, kernel_sp: usize) {
        self.ra = crate::trap_user::trap_return as *const () as usize;

        self.sp = kernel_sp;
        self.s = [0; 12];
    }
}
