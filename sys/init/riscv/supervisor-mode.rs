#![no_std]
#![no_main]

#[unsafe(link_section = ".text._start")]
#[unsafe(no_mangle)]
#[unsafe(naked)]
unsafe extern "C" fn _start() -> ! {
    macro_rules! static_str {
        ($name:ident: [u8; _] = $value:expr) => {
            static $name: [u8; $value.len()] = *$value;
        }
    }
    core::arch::naked_asm!(
        // disable interrupts
        "csrw sie, zero",
        // set initial trap vector
        "lla t0, 3f",
        "csrw stvec, t0",

        // clear BSS
        "lla t0, _bss_start",
        "lla t1, _bss_end",
        "li t2, 0",
        "2:",
        #[cfg(target_pointer_width = "32")]
        "    sw t2, 0(t0)",
        #[cfg(not(target_pointer_width = "32"))]
        "    sd t2, 0(t0)",
        "    addi t0, t0, {word_size}",
        "bltu t0, t1, 2b",

        // set stack pointer
        "lla sp, _stack_end",

        // enter Rust
        "j {}",

        // trampoline to the initial trap handler
        "3:",
        // the stack pointer may be garbage
        "lla sp, _stack_end",
        "csrr a0, scause",
        "j {}",
        sym init,
        sym early_interrupt,
        word_size = const size_of::<usize>(),
    );
}

extern "C" fn early_interrupt(cause: usize) -> ! {
    use core::fmt::Write;
    let _ = writeln!(Console, "fatal interrupt: cause 0x{cause:0x}");
    let error = shutdown();
    let _ = writeln!(Console, "SBI shutdown failed: {error}");
    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}

extern "C" fn init() {
    nucleus::main();
}

struct Console;
impl core::fmt::Write for Console {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        write(s);
        Ok(())
    }
}

fn write(s: &str) -> usize {
    let mut len = s.len();
    let mut ptr = s.as_ptr();

    while len > 0 {
        let error: usize;
        let count: usize;
        unsafe {
            core::arch::asm!(
                "ecall",
                in("a0") len,
                in("a1") ptr,
                in("a2") 0,
                in("a6") 0,
                in("a7") 0x4442434E,
                lateout("a0") error,
                lateout("a1") count,
            );
        }
        if error != 0 {
            return error;
        }
        break;
        len -= count;
        ptr = unsafe { ptr.add(count) };
    }
    0
}

fn shutdown() -> usize {
    let error: usize;
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a0") 0,
            in("a1") 0,
            in("a2") 0,
            in("a6") 0,
            in("a7") 0x53525354,
            lateout("a0") error,
            lateout("a1") _,
        );
    }
    error
}
