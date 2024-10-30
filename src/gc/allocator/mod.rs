use std::alloc::{AllocError, Allocator, Layout};
use std::mem::MaybeUninit;
use std::ptr::NonNull;
use std::sync::{LazyLock, RwLock};

use collector::gc_main;
use thread_local::ThreadLocal;
use tl_allocator::TLAllocator;

use super::os_dependent::{MemorySource, WindowsMemorySource};

mod collector;
mod heap_block_header;
mod tl_allocator;

use collector::DEALLOCATED_CHANNEL;

#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub enum GCAllocatorError {
    ZeroSized,
    AlignmentTooHigh,
    OutOfMemory,
    NoBlocksFound,
}


#[cfg(target_os="windows")]
type MemorySourceImpl = WindowsMemorySource;
static MEMORY_SOURCE: &LazyLock<MemorySourceImpl> = if cfg!(windows) {
    &crate::gc::os_dependent::windows::mem_source::WIN_ALLOCATOR
} else { panic!("Other OS's memory sources") };

static THREAD_LOCAL_ALLOCATORS: RwLock<ThreadLocal<TLAllocator<MemorySourceImpl>>> = RwLock::new(ThreadLocal::new());

pub struct GCAllocator;

impl GCAllocator {
    pub fn allocate_for_type<T>(&self) -> Result<NonNull<MaybeUninit<T>>, GCAllocatorError> {
        let tl_reader = THREAD_LOCAL_ALLOCATORS.read().unwrap();
        let allocator = tl_reader.get_or_try(|| TLAllocator::try_new(MEMORY_SOURCE))?;
        allocator.allocate_for_type::<T>()
    }
    
    /// Return whether or not the GC manages a given piece of data.
    pub fn contains<T: ?Sized>(&self, value: *const T) -> bool {
        MEMORY_SOURCE.contains(value as *const ())
    }
}

unsafe impl Allocator for GCAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.size() == 0 {
            return Err(std::alloc::AllocError) // pls no ZSTs thx
        }
        
        let tl_reader = THREAD_LOCAL_ALLOCATORS.read().unwrap();
        let allocator = tl_reader.get_or_try(|| TLAllocator::try_new(MEMORY_SOURCE)).map_err(|_| AllocError)?;
        
        let (_header, block) = allocator.raw_allocate(layout).map_err(|_| AllocError)?;
        
        Ok(block)
    }
    
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // sanity check
        assert!(ptr.is_aligned_to(layout.align()));
        
        let block = NonNull::from_raw_parts(ptr.cast(), layout.size());
        
        DEALLOCATED_CHANNEL.wait().send(block.into()).expect("The GC thread shouldn't ever exit");
    }
}

fn initialize_logging() {
    use simplelog::*;
    use std::fs::File;
    CombinedLogger::init(
        vec![
            TermLogger::new(LevelFilter::Warn, Config::default(), TerminalMode::Mixed, ColorChoice::Auto),
            WriteLogger::new(LevelFilter::Info, Config::default(), File::create("gc_tests.log").unwrap()),
        ]
    ).unwrap();
}

pub static GC_ALLOCATOR: LazyLock<GCAllocator> = LazyLock::new(|| {
    initialize_logging();
    std::thread::spawn(gc_main);
    GCAllocator
});
