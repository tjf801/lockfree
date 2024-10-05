use std::sync::atomic;



pub struct ConcurrentLinkedList<T> {
    head: LinkedListNode<T>,
}

impl<T> ConcurrentLinkedList<T> {
    fn push_front(&self, element: T) {
        todo!()
    }
}

impl<T: Send> ConcurrentLinkedList<T> {
    fn pop_front(&self) -> Option<T> {
        todo!()
    }
}

impl<T: PartialEq> ConcurrentLinkedList<T> {
    fn contains(&self, element: T) -> bool {
        todo!()
    }
}

struct LinkedListNode<T> {
    next: atomic::AtomicPtr<LinkedListNode<T>>,
    refcnt: atomic::AtomicUsize, // what no gc does to a mf 💔
    value: T
}

impl<T> std::ops::Deref for LinkedListNode<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> LinkedListNode<T> {
    fn push_next(&self, value: T) {
        todo!()
    }
}

impl<T: Send> LinkedListNode<T> {
    fn pop_next(&self) -> Option<T> {
        todo!()
    }
}
