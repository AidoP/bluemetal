#[cfg(target_arch = "riscv64")]
#[no_mangle]
extern "C" fn trap_early_panic(trap_pc: usize, trap_cause: usize) -> ! {
    extern "C" {
        fn _hang() -> !;
    }
    // UART may not have been initialised
    ::serial::init();
    let mut out = ::serial::global().lock();
    let _ = writeln!(
        out,
        r##"
                            __...------------._
                         ,-'                   `-.
                      ,-'                         `.
                    ,'                            ,-`.
                   ;                              `-' `.
                  ;                                 .-. \
                 ;                           .-.    `-'  \
                ;                            `-'          \
               ;                                          `.
               ;                                           :
              ;                                            |
             ;                                             ;
            ;                            ___              ;
           ;                        ,-;-','.`.__          |
       _..;                      ,-' ;`,'.`,'.--`.        |
      ///;           ,-'   `. ,-'   ;` ;`,','_.--=:      /
     |'':          ,'        :     ;` ;,;,,-'_.-._`.   ,'
     '  :         ;_.-.      `.    :' ;;;'.ee.    \|  /
      \.'    _..-'/8o. `.     :    :! ' ':8888)   || /
       ||`-''    \\88o\ :     :    :! :  :`""'    ;;/
       ||         \"88o\;     `.    \ `. `.      ;,'
       /)   ___    `."'/(--.._ `.    `.`.  `-..-' ;--.
       \(.="""""==.. `'-'     `.|      `-`-..__.-' `. `.
        |          `"==.__      )                    )  ;
        |   ||           `"=== '                   .'  .'
        /\,,||||  | |           \                .'   .'
        | |||'|' |'|'           \|             .'   _.' \
        | |\' |  |           || ||           .'    .'    \
        ' | \ ' |'  .   ``-- `| ||         .'    .'       \
          '  |  ' |  .    ``-.._ |  ;    .'    .'          `.
       _.--,;`.       .  --  ...._,'   .'    .'              `.__
     ,'  ,';   `.     .   --..__..--'.'    .'                __/_\
   ,'   ; ;     |    .   --..__.._.'     .'                ,'     `.
  /    ; :     ;     .    -.. _.'     _.'                 /         `
 /     :  `-._ |    .    _.--'     _.'                   |
/       `.    `--....--''       _.'                      |
          `._              _..-'                         |
             `-..____...-''                              |
 ITS A TRAP!                                             |
     pc: 0x{trap_pc:016x}    mGk                       |
  cause: 0x{trap_cause:016x}                              |"##);
    unsafe { _hang() }
}
