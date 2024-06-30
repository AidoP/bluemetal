.section .rodata, "a", %progbits

.global EH_FRAME
EH_FRAME:
    .dword _eh_frame
    .dword _eh_frame_len
