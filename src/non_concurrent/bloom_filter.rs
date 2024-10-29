use std::hash::{BuildHasher, Hash, RandomState};


pub struct BloomFilter<const NUM_HASHES: usize = 5, S: BuildHasher = RandomState> {
    bit_array: Box<[u64]>,
    num_u64s: usize,
    num_elements: usize,
    num_set_bits: usize,
    hashes: [S; NUM_HASHES],
}

impl BloomFilter<5, RandomState> {
    /// Creates a BloomFilter with at least `bits` bits.
    pub fn new(bits: usize) -> Self {
        let hashes = [(); 5].map(|_| std::hash::RandomState::new());
        let num_u64s = bits.div_ceil(64);
        
        Self {
            bit_array: [0].repeat(num_u64s).into_boxed_slice(),
            num_u64s,
            num_elements: 0,
            num_set_bits: 0,
            hashes
        }
    }
}

impl<S: BuildHasher, const NUM_HASHES: usize> BloomFilter<NUM_HASHES, S> {
    /// The amount of elements put into the bloom filter
    pub fn len(&self) -> usize {
        self.num_elements
    }
    
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    
    /// The total amount of bits in the bloom filter.
    pub fn bit_len(&self) -> usize {
        self.num_u64s * 64
    }
    
    /// The (approximate) false positive rate for the bloom filter.
    /// 
    /// This assumes all hash functions are uniform and independent, which may not be true.
    pub fn approx_false_positive_rate(&self) -> f64 {
        let popcnt = self.num_set_bits;
        (popcnt as f64 / self.bit_len() as f64).powi(NUM_HASHES as i32)
    }
    
    /// Inserts a value into the bloom filter.
    pub fn add<T: ?Sized + Hash>(&mut self, value: &T) {
        for h in &self.hashes {
            let hash = h.hash_one(value);
            let (word, bit) = (hash / 64, hash % 64);
            let index = word as usize % self.num_u64s;
            
            self.num_set_bits += ((self.bit_array[index] >> bit) & 1) as usize;
            self.bit_array[index] |= 1 << bit;
        }
        self.num_elements += 1;
    }
    
    /// Whether the bloom filter might contain `value`.
    /// 
    /// This function may return false positives, but will never return false negatives.
    pub fn contains<T: ?Sized + Hash>(&self, value: &T) -> bool {
        for h in &self.hashes {
            let hash = h.hash_one(value);
            let (word, bit) = (hash / 64, hash % 64);
            let index = word as usize % self.num_u64s;
            
            if self.bit_array[index] & (1 << bit) == 0 {
                return false
            }
        }
        true
    }
}

#[test]
fn basic_test() {
    let mut bf = BloomFilter::new(64);
    
    bf.add("hello");
    bf.add("world");
    bf.add("foo");
    bf.add("bar");
    assert!(bf.contains("hello"));
    assert!(!bf.contains("baz"));
    println!("{}", bf.approx_false_positive_rate());
    for i in 0..10000 {
        if bf.contains(&i) {
            println!("{i}");
        }
    }
}

