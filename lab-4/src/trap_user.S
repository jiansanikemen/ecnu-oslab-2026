    .section .text.trampoline
    .align 12
    .globl user_vector
    .globl user_restore

user_vector:
    # 进入前：
    # sp = 用户栈指针
    # sscratch = TRAP_CONTEXT
    # 仍使用用户页表
    csrrw sp, sscratch, sp

    # 此时 sp 指向用户 TrapContext。
    # 先保存寄存器，再使用临时寄存器。
    sd zero,   0(sp)
    sd ra,     8(sp)
    sd gp,    24(sp)
    sd tp,    32(sp)
    sd t0,    40(sp)
    sd t1,    48(sp)
    sd t2,    56(sp)
    sd s0,    64(sp)
    sd s1,    72(sp)
    sd a0,    80(sp)
    sd a1,    88(sp)
    sd a2,    96(sp)
    sd a3,   104(sp)
    sd a4,   112(sp)
    sd a5,   120(sp)
    sd a6,   128(sp)
    sd a7,   136(sp)
    sd s2,   144(sp)
    sd s3,   152(sp)
    sd s4,   160(sp)
    sd s5,   168(sp)
    sd s6,   176(sp)
    sd s7,   184(sp)
    sd s8,   192(sp)
    sd s9,   200(sp)
    sd s10,  208(sp)
    sd s11,  216(sp)
    sd t3,   224(sp)
    sd t4,   232(sp)
    sd t5,   240(sp)
    sd t6,   248(sp)

    # sscratch 中保存着交换前的用户 sp。
    csrr t0, sscratch
    sd t0, 16(sp)

    csrr t0, sstatus
    sd t0, 256(sp)

    csrr t0, sepc
    sd t0, 264(sp)

    # 在切换页表前，取出所有内核返回信息。
    ld t0, 272(sp)       # kernel_satp
    ld t1, 280(sp)       # kernel_sp
    ld t2, 288(sp)       # trap_handler

    csrw satp, t0
    sfence.vma zero, zero

    # 此后使用内核地址空间和独立内核栈。
    mv sp, t1

    # 本实验只在 hart 0 运行首个用户进程。
    # 用户 tp 不可信，进入 Rust 前恢复内核 hart ID。
    li tp, 0

    jr t2

user_restore:
    # 参数：
    # a0 = TRAP_CONTEXT 虚拟地址
    # a1 = 用户页表 token
    # 当前仍在内核页表下执行。
    csrw satp, a1
    sfence.vma zero, zero

    # 切换后才能通过用户页表访问 TRAP_CONTEXT。
    mv sp, a0

    # 为下一次用户 Trap 准备交换地址。
    csrw sscratch, sp

    ld t0, 256(sp)
    ld t1, 264(sp)
    csrw sstatus, t0
    csrw sepc, t1

    ld ra,     8(sp)
    ld gp,    24(sp)
    ld tp,    32(sp)
    ld t0,    40(sp)
    ld t1,    48(sp)
    ld t2,    56(sp)
    ld s0,    64(sp)
    ld s1,    72(sp)
    ld a0,    80(sp)
    ld a1,    88(sp)
    ld a2,    96(sp)
    ld a3,   104(sp)
    ld a4,   112(sp)
    ld a5,   120(sp)
    ld a6,   128(sp)
    ld a7,   136(sp)
    ld s2,   144(sp)
    ld s3,   152(sp)
    ld s4,   160(sp)
    ld s5,   168(sp)
    ld s6,   176(sp)
    ld s7,   184(sp)
    ld s8,   192(sp)
    ld s9,   200(sp)
    ld s10,  208(sp)
    ld s11,  216(sp)
    ld t3,   224(sp)
    ld t4,   232(sp)
    ld t5,   240(sp)
    ld t6,   248(sp)

    # 最后恢复用户 sp，否则会失去上下文基地址。
    ld sp, 16(sp)

    sret
