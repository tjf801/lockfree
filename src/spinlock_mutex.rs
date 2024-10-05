use std::sync::atomic::{AtomicBool, Ordering};
use std::cell::UnsafeCell;

// following along with https://www.youtube.com/watch?v=rMGWeSjctlY
pub struct Mutex<T> {
    locked : AtomicBool,
    v : UnsafeCell<T>
}

impl<T> Mutex<T> {
    pub fn new(t : T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            v: UnsafeCell::new(t)
        }
    }
    
    // https://matklad.github.io/2020/01/02/spinlocks-considered-harmful.html
    pub fn with_lock<F, R>(&self, f: F) -> R where F: FnOnce(&mut T) -> R {
        while self.locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            std::hint::spin_loop();
            std::thread::yield_now();
            
            // this is here because of the [MESI protocol](https://en.wikipedia.org/wiki/MESI_protocol) ... or something ?
            while self.locked.load(Ordering::Relaxed) {
                std::hint::spin_loop();
                std::thread::yield_now();
            }
            
            // compare_exchange vs compare_exchange_weak:
            //   - x.compare_exchange(a, ...) only fails if x ≠ a
            //   - x.compare_exchange_weak(a, ...) can fail even when x = a
        }
        
        // SAFETY: cast into &mut is safe because no other thread has access to the `T`, since only this thread holds the lock.
        //         This also must happen AFTER we aquire the lock, and BEFORE we release the lock, because of the mem orderings.
        let ret = f(unsafe { &mut *self.v.get() } );
        
        // store(Release) → everything that happens earlier on this thread is seen by any load(Aquire+)
        self.locked.store(false, Ordering::Release);
        
        ret
    }
}

unsafe impl<T> Sync for Mutex<T> where T: Send {}

#[cfg(test)]
mod tests {
    use super::*;
    
    // https://doc.rust-lang.org/nightly/nomicon/atomics.html
    //     Asking for guarantees that are too weak on strongly-ordered hardware is more likely to happen to work, even though your program is strictly incorrect.
    //     If possible, concurrent algorithms should be tested on weakly-ordered hardware.
    // mfw im on (strongly ordered) x86
    
    #[test]
    fn mutex_usize() {
        use std::thread;
        const T: usize = 100;
        const R: usize = 1000;
        
        let m = Box::leak(Box::new(Mutex::new(0)));
        
        let handles = (0..T).map(|_| 
            thread::spawn(|| 
                for _ in 0..R {
                    m.with_lock(|v| *v += 1)
                }
            )
        ).collect::<Vec<_>>();
        
        for handle in handles {
            handle.join().unwrap();
        }
        
        assert_eq!(m.with_lock(|v| *v), T*R);
    }
    
    #[test]
    fn mutex_vec_push() {
        use std::thread;
        const T: usize = 100;
        const R: usize = 1000;
        
        let m = Box::leak(Box::new(Mutex::new(vec![])));
        
        let handles = (0..T).map(|_| 
            thread::spawn(|| 
                for _ in 0..R {
                    m.with_lock(|v| v.push(v.len()))
                }
            )
        ).collect::<Vec<_>>();
        
        for handle in handles {
            handle.join().unwrap();
        }
        
        assert_eq!(m.with_lock(|v| v.len()), T*R);
    }
}
