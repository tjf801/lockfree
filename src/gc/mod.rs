pub mod allocator;
mod smart_pointers;

pub use smart_pointers::{Gc, GcMut};
use std::arch::asm;

#[cfg(target_arch="x86_64")]
fn stack_base() -> *const usize {
    let mut stack_base: *const usize = 0 as _;
    unsafe {
        asm!(
            "mov {0}, rbp",
            out(reg) stack_base,
        )
    };
    
    loop {
        println!("{:x?}", stack_base);
        // SAFETY: on x86, `rbp` contains the address of the previous `rbp`, all the way up the call stack.
        //         this is still *probably* a violation of an alias guaruntee of rust's, but the opsem
        //         around `volatile` arent really figured out yet so... this is probably fine lol
        let x = unsafe { stack_base.read_volatile() } as *const usize;
        if !x.is_aligned() || x.is_null() {
            break stack_base;
        }
        stack_base = x;
    }
}

// test if any value on the stack has a given value
#[cfg(all(target_arch="x86_64", target_os="windows"))]
fn stack_scan(value: usize) -> bool {
    let mut current: *const usize = current_stack_pointer();
    let base = stack_base();
    while current <= base {
        let x = unsafe { current.read_volatile() };
        println!("[{current:x?}]: {x:016x}");
        // if x == value {
        //     return true
        // }
        current = current.wrapping_offset(1);
    }
    
    false
}


#[test]
fn test() {
    let mut x = 0x694205A5A5A5A5A5usize;
    
    // println!("{:?}", &raw const x);
    
    let x = stack_base();
    println!("stack base: {x:x?}");
    
    assert!(stack_scan(0x694205A5A5A5A5A5usize));
    
    println!("{:?}", &raw const x);
}

#[cfg(test)]
mod tests {
    use crate::gc::stack_scan;
    
    #[test]
    fn test() {
        let x = 0x694205A5A5A5A5A5usize;
        std::hint::black_box(&raw const x);
        
        assert!(stack_scan(0x694205A5A5A5A5A5usize))
    }
}
