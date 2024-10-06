
pub mod allocator;
pub mod os_dependent;

mod smart_pointers;

// re-export the `Gc` and `GcMut` smart pointers, they are the main API to use
pub use smart_pointers::{Gc, GcMut};


