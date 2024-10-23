use std::mem::MaybeUninit;
use std::ptr::NonNull;
use std::alloc::{Allocator, Layout};
use std::sync::LazyLock;

use super::os_dependent::windows::mem_source::WindowsMemorySource;
use super::os_dependent::MemorySource;


// TODO: tri-color allocations
pub struct GCAllocator<M: 'static + Sync> {
    memory_source: &'static M,
    // heap_lock: std::sys::sync::Mutex,
}

// TODO: actually make this thread-safe lmao
unsafe impl<M: Sync> Send for GCAllocator<M> {}
unsafe impl<M: Sync> Sync for GCAllocator<M> {}

#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub enum GCAllocatorError {
    ZeroSized,
    AlignmentTooHigh,
    OutOfMemory,
}

type HeaderFlag = usize;
const HEADERFLAG_ALLOCATED: HeaderFlag = 0x01;
const HEADERFLAG_OK_TO_DROP: HeaderFlag = 0x02;
const HEADERFLAG_DROPPED: HeaderFlag = 0x04;

const PTR_METADATA_SIZE: usize = usize::BITS as usize / 8;
type PtrMeta = [u8; PTR_METADATA_SIZE];
type Dropper = unsafe fn(*mut (), *const PtrMeta);

#[repr(C, align(16))]
struct GCObjectHeader {
    next: Option<NonNull<GCObjectHeader>>,
    flags: HeaderFlag,
    /// SAFETY: this function must be `std::ptr::drop_in_place::<T>` where `T` is the the object that this header is for.
    dropper: Option<Dropper>,
    /// The metadata for the pointer which gets dropped.
    /// 
    /// e.g: for a GCObjectHeader for a `[i32]`, this field contains (the bytes of) the length of the slice.
    ptr_meta_bytes: PtrMeta,
}

/// basic wrapper around `std::ptr::drop_in_place` to take whatever kinda pointer, including fat ones
unsafe fn dropper<T: ?Sized>(value: *mut (), ptr_meta_bytes: *const PtrMeta) {
    use std::ptr::Pointee;
    use std::mem::size_of;
    
    // statically assert that the pointer metadata will fit into the allotted space.
    // if this is not true, something has gone horribly wrong. (and i need to fix it)
    const fn const_assert(expr: bool) { assert!(expr) }
    const_assert(size_of::<<T as Pointee>::Metadata>() <= size_of::<PtrMeta>());
    
    // SAFETY: casting `metadata_ptr` as `T::Metadata` is valid because [TODO]
    let metadata = unsafe { *(ptr_meta_bytes as *const <T as Pointee>::Metadata) };
    let t_ptr: *mut T = std::ptr::from_raw_parts_mut(value, metadata);
    
    // SAFETY: TODO (forward requirements to caller)
    unsafe { std::ptr::drop_in_place(t_ptr) }
}

impl<M> GCAllocator<M> where M: Sync + MemorySource {
    fn new() -> Self {
        todo!()
    }
    
    /// 
    /// 
    /// The returned header contains `Default::default` for the `dropper` and `ptr_metadata` fields (i.e: `None` and `[0; 8]`).
    /// This means that the given value does NOT drop unless these fields are initialized!!!
    unsafe fn raw_allocate(&self, layout: Layout) -> Result<(NonNull<GCObjectHeader>, NonNull<[u8]>), GCAllocatorError> {
        todo!()
    }
    
    /// allocates a block to store `layout`, and initializes it with the necessary metadata for the GC to drop it later.
    unsafe fn raw_allocate_with_drop(&self, layout: Layout, dropper: Dropper, ptr_metadata: PtrMeta) -> Result<NonNull<[u8]>, GCAllocatorError> {
        let (block, data) = unsafe { self.raw_allocate(layout)? };
        
        // SAFETY: dropper field is in bounds of the allocation, afaik this is similar to using `ptr::offset`
        let dropper_ptr = unsafe { &raw mut (*block.as_ptr()).dropper };
        // SAFETY: its fine to write here
        unsafe { dropper_ptr.write(Some(dropper)) };
        
        // SAFETY: same as above
        let ptr_meta_ptr = unsafe { &raw mut (*block.as_ptr()).ptr_meta_bytes };
        // SAFETY: ok to write
        unsafe { ptr_meta_ptr.write(ptr_metadata) };
        
        Ok(data)
    }
    
    pub fn allocate_for_type<T: Send>(&self) -> Result<NonNull<MaybeUninit<T>>, GCAllocatorError> {
        let dropper = dropper::<T>;
        let type_layout = std::alloc::Layout::new::<T>();
        // using default is fine here. since `<*const T>::Metadata` is `()`, it literally doesnt matter
        let result = unsafe { self.raw_allocate_with_drop(type_layout, dropper, Default::default()) }?;
        
        // sanity check
        // SAFETY: length of slice is initialized, and whole slice fits in `isize`
        assert_eq!(unsafe { std::mem::size_of_val_raw(result.as_ptr()) }, std::mem::size_of::<T>());
        
        Ok(result.cast::<MaybeUninit<T>>())
    }
    
    pub fn allocate_for_slice<T: Send>(&self, len: usize) -> Result<NonNull<[MaybeUninit<T>]>, GCAllocatorError> {
        // helper function to get the layout of a slice
        fn get_slice_layout<T>(len: usize) -> Result<Layout, GCAllocatorError> {
            // make sure `len * sizeof(T)` fits in `isize`
            isize::try_from(len * std::mem::size_of::<T>()).map_err(|_| GCAllocatorError::OutOfMemory)?;
            let ptr: *const [T] = std::ptr::from_raw_parts(std::ptr::null::<T>(), len);
            // SAFETY: `len` metadata is initialized and fits in `isize`
            Ok(unsafe { Layout::for_value_raw(ptr) })
        }
        
        let layout = get_slice_layout::<T>(len)?;
        // SAFETY: its just a slice of bytes, im sure its ok :3
        let metadata = unsafe { std::mem::transmute_copy(&len) };
        let result = self.raw_allocate_with_drop(layout, dropper::<[T]>, metadata)?;
        
        // SAFETY: length of slice is initialized, and whole slice fits in `isize`
        assert_eq!(unsafe { std::mem::size_of_val_raw(result.as_ptr()) }, std::mem::size_of::<T>());
        
        Ok(NonNull::from_raw_parts(result.cast(), len))
    }
    
    /// Return whether or not the GC manages a given piece of data.
    pub fn contains<T: ?Sized>(&self, value: *const T) -> bool {
        self.memory_source.contains(value as *const ())
    }
}

unsafe impl<M> Allocator for GCAllocator<M> where M: Sync + MemorySource {
    /// NOTE: calling this directly will not initialize a destructor to run!!!
    /// (But usually the allocator API is only called in contexts where the destructor will run otherwise, so its whatever)
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, std::alloc::AllocError> {
        if layout.size() == 0 {
            return Err(std::alloc::AllocError) // pls no ZSTs thx
        }
        let (_header, data) = unsafe { self.raw_allocate(layout).map_err(|_e| std::alloc::AllocError) }?;
        Ok(data)
    }
    
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // if we get here, we can add `ptr` to the "definitely able to free" list.
        todo!()
    }
}

#[cfg(target_os="windows")]
pub static GC_ALLOCATOR: LazyLock<GCAllocator<WindowsMemorySource>> = LazyLock::new(GCAllocator::new);
