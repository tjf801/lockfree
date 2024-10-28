use std::ptr::Unique;
use std::sync::{mpsc, OnceLock};

pub(super) static DEALLOCATED_CHANNEL: OnceLock<mpsc::Sender<std::ptr::Unique<[u8]>>> = OnceLock::new();

pub(super) fn gc_main() -> ! {
    let (sender, reciever) = mpsc::channel::<Unique<[u8]>>();
    DEALLOCATED_CHANNEL.set(sender).expect("Nobody but here sets `DEALLOCATED_CHANNEL`");
    
    loop {
        let to_free = reciever.recv().expect("Sender is stored with 'static lifetime");
        error!("TODO: free heap block at {:016x?}", to_free);
    }
}
