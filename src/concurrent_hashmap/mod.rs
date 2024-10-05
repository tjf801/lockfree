use std::borrow::Borrow;
use std::cell::UnsafeCell;
use std::hash::Hash;
use std::marker::PhantomData;

const MAX_CAPACITY: usize = i32::MAX as usize;
const DEFAULT_CAPACITY: usize = 16;

const DEFAULT_LOAD_FACTOR: f32 = 0.75;

// following along with https://www.youtube.com/watch?v=yQFWmGaFBjk
struct ConcurrentHashMap<K, V, H = std::collections::hash_map::RandomState> {
    todo: PhantomData<(K, V)>,
    hasher: H
}

impl<K, V, H> ConcurrentHashMap<K, V, H> {
    fn new() -> Self {
        todo!()
    }
    
    fn with_capacity(capacity: usize) -> Self {
        todo!()
    }
    
    fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K : Borrow<Q>,
        Q : ?Sized + Hash + Eq
    {
        todo!()
    }
    
    fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K : Borrow<Q>,
        Q : ?Sized + Hash + Eq
    {
        todo!()
    }
    
    fn insert(&self, key: K, value: V) -> Option<V> {
        
        todo!()
    }
    
    fn remove<Q>(&self, key: &Q) -> Option<V>
    where
        K : Borrow<Q>,
        Q : ?Sized + Hash + Eq
    {
        todo!()
    }
    
    pub fn remove_entry<Q>(&self, key: &Q) -> Option<(K, V)>
    where
        K: Borrow<Q>,
        Q: ?Sized + Hash + Eq
    {
        todo!()
    }
}
