#![allow(unused)]
#![deny(unsafe_op_in_unsafe_fn)]
#![feature(allocator_api)]

#![feature(const_trait_impl)]

// for const `TakeCell::into_inner`
// (alternatively, a const `DerefOwned` impl fingers crossed)
#![feature(const_precise_live_drops)]
#![feature(const_cell_into_inner)]
#![feature(const_unsafecell_get_mut)]
#![feature(deref_pure_trait)]
#![feature(sync_unsafe_cell)]

#![feature(array_windows)]

extern crate windows_sys;

// not concurrent
mod non_concurrent;

// concurrency primitives
mod cell;
mod atomic_refcount;
mod spinlock_mutex;

// garbage collection
mod gc;

// concurrent data structures
mod concurrent_vec;
mod concurrent_hashmap;
mod concurrent_linkedlist;

// NOTE: a type is `Send` if the thread that created it doesn't need to be the one that frees it
// NOTE: a type is `Sync` if its safe to share multiple references to it between threads at once
