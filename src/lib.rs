#![allow(internal_features)]
#![warn(unsafe_op_in_unsafe_fn)]

// Language features
#![feature(let_chains)]
#![feature(coroutines)]
#![feature(negative_impls)]
#![feature(gen_blocks)]
#![feature(arbitrary_self_types_pointers)]
#![feature(dropck_eyepatch)]
#![feature(const_precise_live_drops)]

// AAAA. `std::sys` has so many good abstractions i would like to use, but its private and i cant find ANY features that make it. not private. fml
#![feature(libstd_sys_internals)]
#![feature(windows_c)]

// Pointers and provenance
#![feature(strict_provenance)]
#![feature(strict_provenance_atomic_ptr)]
#![warn(fuzzy_provenance_casts)]

// New types & traits
#![feature(never_type)]
#![feature(sync_unsafe_cell)]
#![feature(allocator_api)]
#![feature(deref_pure_trait)]
#![feature(ptr_internals)] // for Unique<T>
#![feature(ptr_metadata)]
#![feature(unsize)]
#![feature(coerce_unsized)]
#![feature(dispatch_from_dyn)]

// Specific methods
#![feature(array_windows)]
#![feature(cell_update)]
#![feature(layout_for_ptr)] // std::mem::size_of_val_raw
#![feature(pointer_is_aligned_to)]
#![feature(box_as_ptr)]
#![feature(once_wait)]
#![feature(vec_push_within_capacity)]
#![feature(str_from_raw_parts)]
#![feature(const_cell_into_inner)]
#![feature(const_unsafecell_get_mut)]
#![feature(clone_to_uninit)]


#[macro_use] extern crate log;
extern crate windows_sys;
extern crate simplelog;
extern crate thread_local;

// not concurrent
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
