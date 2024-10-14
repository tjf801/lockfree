// #![allow(unused)]
#![deny(unsafe_op_in_unsafe_fn)]
#![feature(allocator_api)]

#![feature(const_trait_impl)]
#![feature(const_alloc_layout)]
#![feature(const_precise_live_drops)]
#![feature(const_cell_into_inner)]
#![feature(const_unsafecell_get_mut)]

#![feature(deref_pure_trait)]
#![feature(sync_unsafe_cell)]
#![feature(negative_impls)]

// a bunch of stuff related to unsized types
#![feature(unsize)]
#![feature(coerce_unsized)]
#![feature(dispatch_from_dyn)]
#![feature(clone_to_uninit)]
#![feature(layout_for_ptr)]
#![feature(ptr_metadata)]

#![feature(array_windows)]
#![feature(gen_blocks)]

extern crate windows_sys;

// not concurrent
pub mod non_concurrent;

// concurrency primitives
pub mod cell;
pub mod atomic_refcount;
pub mod spinlock_mutex;

// garbage collection
pub mod gc;

// concurrent data structures
pub mod concurrent_vec;
pub mod concurrent_hashmap;
pub mod concurrent_linkedlist;
