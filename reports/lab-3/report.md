# Lab 3：中断与异常初步

## 一、实验目的

在 Lab 2 已启用分页的内核上建立 S-mode Trap 处理机制，实现 UART 外部中断和周期时钟通知，理解寄存器现场保存与恢复、中断委托及多 hart 初始化顺序。

## 二、实验环境

- Ubuntu 24.04 LTS，VMware Workstation Pro 17
- Rust 1.98.1，目标：`riscv64gc-unknown-none-elf`
- QEMU 8.2.2，`virt` 平台，双 hart、128 MiB 内存

## 三、主要实现内容

1. 接入课程提供的 `trap.S`，通过 `KernelTrapContext` 对应的 256 字节布局保存通用寄存器；配置 `medeleg`、`mideleg` 和 `stvec`，处理后通过 `sret` 恢复执行。
2. 在 `plic.rs` 中配置 UART IRQ 10 的优先级，只向 hart 0 的 S-context 使能该中断，实现 claim/complete；在 `memory_set.rs` 中增加 PLIC 三页的 R/W 恒等映射和权限检查。
3. 在 `uart.rs` 中启用接收中断，循环读取接收 FIFO，实现普通字符回显、退格擦除和换行；输入 `K` 时仅输出一次验收标志。
4. 在 `timer.rs` 中为两个 hart 分别配置 `mtimecmp`、`mscratch` 和 `mtvec`。M-mode 时钟入口预约下一次事件并置位 SSIP，S-mode 清除 SSIP，仅由 hart 0 原子更新全局 `ticks`。
5. 在 `main.rs` 中按分页、PLIC 全局初始化、每 hart Trap 初始化、UART 接收使能的顺序接入功能，使用 Release/Acquire 原子同步；未实现的 Trap 输出诊断后停止。

## 四、实验过程与结果

执行 `cargo fmt --check`、`make build` 和 `make qemu`。两个 hart 均输出 `traps enabled`，上下文布局检查通过，时钟计数达到 `ticks=3`。手工输入普通字符、Backspace 和换行均符合预期，输入 `K` 后出现 UART 验收标志。

执行 `make clean && make check`，静态指令检查和 QEMU 集成检查通过，输出包含：

```text
lab-3 context test: PASS
lab-3 timer test: PASS ticks=3
lab-3 uart test: PASS byte=K
lab-3 check: PASS
```

## 五、实验截图

![Lab 3 双 hart 启动与 UART 手动输入测试](assets/lab3-qemu.png)

![Lab 3 自动检查结果](assets/lab3-check.png)

## 六、实验总结

本实验打通了 UART 经 PLIC 到 S-mode 的外部中断路径，以及机器时钟通过 SSIP 通知 S-mode 的路径。实践表明，设备页映射、每 hart 独立现场、中断初始化顺序和挂起位清除是正确处理中断的关键；claim/complete 配对与周期更新比较值保证了后续事件能够继续处理。
