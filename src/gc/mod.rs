
pub mod allocator;
pub mod os_dependent;

mod smart_pointers;

// re-export the `Gc` and `GcMut` smart pointers, they are the main API to use
pub use smart_pointers::{Gc, GcMut};

#[cfg(test)]
mod _tests {
    #[test]
    fn _initialize_logging() {
        use simplelog::*;
        use std::fs::File;
        CombinedLogger::init(
            vec![
                TermLogger::new(LevelFilter::Warn, Config::default(), TerminalMode::Mixed, ColorChoice::Auto),
                WriteLogger::new(LevelFilter::Debug, Config::default(), File::create("gc_tests.log").unwrap()),
            ]
        ).unwrap();
    }
}
