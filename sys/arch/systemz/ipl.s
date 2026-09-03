.global __io_interrupt_code
.global __io_interrupt_new_psw
.global __io_interrupt_old_psw
.global __program_interrupt_new_psw
.global __program_interrupt_new_psw
.global start

.section .start, "x", @progbits
    // initial short-format PSW
    // - bit 12 must be 1
    // - machine check mask on
    // - 31bit addressing
    .4byte 0x000C0000
    .2byte 0x8000
    .2byte start

.org 184
__io_interrupt_code:
    // location of a 12-byte area stored during an I/O interruption

.org 368
__io_interrupt_old_psw:
    // location of the previous PSW after an I/O interruption

.org 464
__program_interrupt_new_psw:
    // initial program interruption new-PSW
    .4byte 0x00040001
    .4byte 0x80000000
    .8byte page_clear_interrupt

.org 496
__io_interrupt_new_psw:
    // initial I/O interruption new-PSW
    .4byte 0x00040001
    .4byte 0x80000000
    .8byte 0

// skip prefix area reserved for the hardware
.org 8192
start:

    // switch to z/Architecture mode
    la 0,0
    la 1,1
    sigp 1,0,18

    // switch to 64-bit addressing
    sam64

    // reset memory and determine total size
    larl 2,__memory_start
    // clear test-block option bits
    lghi 0,0
clear_loop:
    // clear page
    ahi 2,4096
    tb 0,2

    // loop until block not usable
    jz clear_loop

page_clear_interrupt:
    // clear interrupt handler
    larl 1,__program_interrupt_new_psw
    xc 0(16,1),0(1)

    // start stack at the end of real memory, up to the 31-bit boundary
    lgr 15,2
    aghi 15,-160
    clgfi 15,0x80000000
    jl 2f
    llgfi 15,0x80000000-160
2:

    // enable vector instructions and the additional floating-point registers
    stctg 0,0,8(15)
    oi 13(15),0x03 << 1
    lctlg 0,0,8(15)

    // begin main
    j {main}
