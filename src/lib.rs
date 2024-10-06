// #![allow(unused)]
#![deny(unsafe_op_in_unsafe_fn)]
#![feature(allocator_api)]

#![feature(const_trait_impl)]
#![feature(const_alloc_layout)]

// for const `TakeCell::into_inner`
// (alternatively, a const `DerefOwned` impl fingers crossed)
#![feature(const_precise_live_drops)]
#![feature(const_cell_into_inner)]
#![feature(const_unsafecell_get_mut)]
#![feature(deref_pure_trait)]
#![feature(sync_unsafe_cell)]

#![feature(unsize)]
#![feature(coerce_unsized)]
#![feature(dispatch_from_dyn)]

#![feature(array_windows)]

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

// NOTE: a type is `Send` if the thread that created it doesn't need to be the one that frees it
// NOTE: a type is `Sync` if its safe to share multiple references to it between threads at once
