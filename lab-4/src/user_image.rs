const CODE_OFFSET: usize = 0x100;
const CODE_SIZE: usize = 16;
const ELF_SIZE: usize = CODE_OFFSET + CODE_SIZE;

const fn write_le(image: &mut [u8; ELF_SIZE], offset: usize, value: u64, width: usize) {
    let mut index = 0;

    while index < width {
        image[offset + index] = (value >> (index * 8)) as u8;

        index += 1;
    }
}

const fn build_user_elf() -> [u8; ELF_SIZE] {
    let mut image = [0u8; ELF_SIZE];

    // ELF 魔数。
    image[0] = 0x7f;
    image[1] = b'E';
    image[2] = b'L';
    image[3] = b'F';

    image[4] = 2; // ELF64。
    image[5] = 1; // 小端。
    image[6] = 1; // ELF 版本。

    // ELF64 文件头。
    write_le(&mut image, 16, 2, 2); // ET_EXEC。
    write_le(&mut image, 18, 243, 2); // RISC-V。
    write_le(&mut image, 20, 1, 4); // e_version。

    write_le(&mut image, 24, crate::process::USER_ENTRY as u64, 8);

    write_le(&mut image, 32, 64, 8); // 程序头表偏移。
    write_le(&mut image, 52, 64, 2); // ELF 文件头大小。
    write_le(&mut image, 54, 56, 2); // 程序头大小。
    write_le(&mut image, 56, 1, 2); // 一个程序头。

    // 第一个程序头从文件偏移 64 开始。
    write_le(&mut image, 64, 1, 4); // PT_LOAD。
    write_le(&mut image, 68, 5, 4); // PF_R | PF_X。

    write_le(&mut image, 72, CODE_OFFSET as u64, 8);

    write_le(&mut image, 80, crate::process::USER_ENTRY as u64, 8);

    // p_paddr：用户装载器不使用此字段。
    write_le(&mut image, 88, 0, 8);

    write_le(&mut image, 96, CODE_SIZE as u64, 8);

    // 多留 16 字节，便于检查文件数据之后保持清零。
    write_le(&mut image, 104, (CODE_SIZE + 16) as u64, 8);

    // 文件偏移和虚拟地址均满足 0x100 对齐约束。
    write_le(&mut image, 112, 0x100, 8);

    // 四条 32 位 RISC-V 指令。
    write_le(&mut image, CODE_OFFSET, 0x0010_0893, 4);
    write_le(&mut image, CODE_OFFSET + 4, 0x0000_0073, 4);
    write_le(&mut image, CODE_OFFSET + 8, 0x0000_0073, 4);
    write_le(&mut image, CODE_OFFSET + 12, 0x0000_006f, 4);

    image
}

static USER_ELF_BYTES: [u8; ELF_SIZE] = build_user_elf();

pub static USER_ELF: &[u8] = &USER_ELF_BYTES;
