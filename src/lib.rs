#![allow(internal_features)]
#![deny(unsafe_op_in_unsafe_fn)]
#![feature(never_type)]
#![feature(allocator_api)]

#![feature(const_trait_impl)]
#![feature(const_alloc_layout)]
#![feature(const_precise_live_drops)]
#![feature(const_cell_into_inner)]
#![feature(const_unsafecell_get_mut)]

#![feature(deref_pure_trait)]
#![feature(dropck_eyepatch)]
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

#![feature(strict_provenance)]
#![feature(strict_provenance_atomic_ptr)]
#![warn(fuzzy_provenance_casts)]
#![feature(arbitrary_self_types_pointers)]

#![feature(windows_c)]
// AAAA. `std::sys` has so many good abstractions i would like to use, but its private and i cant find ANY features that make it. not private. fml
#![feature(libstd_sys_internals)]

#[macro_use] extern crate log;
extern crate windows_sys;
extern crate simplelog;

// not concurrent
#[allow(unused)]
pub mod non_concurrent;

// concurrency primitives
pub mod cell;
pub mod atomic_refcount;
pub mod spinlock_mutex;

// garbage collection
pub mod gc;

// concurrent data structures
#[allow(unused)]
pub mod concurrent_vec;
#[allow(unused)]
pub mod concurrent_hashmap;
#[allow(unused)]
pub mod concurrent_linkedlist;
