/// Suffix Array Data Structure
pub struct SuffixArray<'a> {
    // NOTE: these are both O(n) space!
    suffixes: Box<[&'a str]>, // NOTE: borrowed string references are just (ptr, len) pairs, and don't store any of the actual string
    lcp_array: Box<[usize]>,
}

impl<'a> SuffixArray<'a> {
    /// Complexity: O(n log(n))
    /// 
    /// TODO: O(n) complexity at https://arxiv.org/abs/1610.08305
    pub fn new(string: &'a str) -> Self {
        let mut suffixes = Vec::from_iter((0..string.len()).map(|i| &string[i..]));
        suffixes.sort();
        
        // TODO: this is not idiomatic
        let lcp_array = suffixes.array_windows::<2>().map(|&[a, b]| {
            let mut i = 0;
            let mut x = a.bytes();
            let mut y = b.bytes();
            while x.next() == y.next() { i += 1 }
            i
        }).collect();
        
        Self {
            suffixes: suffixes.into(),
            lcp_array
        }
    }
    
    /// Complexity: O(log(n))
    pub fn is_suffix(&self, value: &str) -> bool {
        self.suffixes.binary_search(&value).is_ok()
    }
    
    /// Complexity: O(log(n))
    pub fn has_substring(&self, value: &str) -> bool {
        match self.suffixes.binary_search(&value) {
            Ok(_) => true, // not just any substring, but a suffix
            Err(idx) => {
                // `suffix_idxes[idx]` is the suffix where `value` would be a prefix, if any
                self.suffixes[idx].strip_prefix(value).is_some()
            }
        }
    }
    
    /// Complexity: O(n)
    pub fn longest_repeated_substring(&self) -> Option<&'a str> {
        let (idx, &len) = self.lcp_array.iter().enumerate().max_by_key(|&(_, a)| a)?;
        if len == 0 { return None }
        Some(&self.suffixes[idx][..len])
    }
    
    pub fn shortest_non_repeated_substring(&self) -> Option<&'a str> {
        // min of pairwise maxes of lcp array values
        let (len, idx) = self.suffixes.iter().enumerate().skip(1).map(|(i, &v)| {
            let x = self.lcp_array[i-1];
            let y = *self.lcp_array.get(i).unwrap_or(&0);
            let l = std::cmp::max(x, y);
            if l == v.len() { return (usize::MAX, i) }
            (l, i)
        }).min_by_key(|&(l, _)| l)?;
        Some(&self.suffixes[idx][..=len])
    }
}

#[test]
fn doesitwork() {
    let x = SuffixArray::new("CGTATGCGGCATGCTAGCTAGGCGTGTAGTGCTGGAGGTTTTTCGGATCGTAGCTAGTGCGTGTATTCAGTTTATTAATTATAATATCGAGTCGTGCAGTCGTACATGCATGCTGCA");
    println!("{:?}", x.longest_repeated_substring());
    println!("{:?}", x.shortest_non_repeated_substring());
    println!("{:?}", x.has_substring("TGCTGA"));
}

