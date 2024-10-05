use std::ptr::NonNull;



struct RBTree {
    
}

// PROVE: any node with height `h` has black height at least `h/2`
// PROVE: the subtree located at any node `x` contains at least `2^bh(x) - 1` nodes (use induction)
// LEMMA: An RBTree with `n` internal nodes has height at most `2*log₂(n+1)`

struct RBTreeNode<T> {
    color: bool,
    value: T,
    left: Option<NonNull<RBTreeNode<T>>>,
    right: Option<NonNull<RBTreeNode<T>>>,
}


