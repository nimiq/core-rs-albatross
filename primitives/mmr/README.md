# MMR

Merkle Mountain Ranges are an alternative to Merkle trees. While the latter relies on perfectly balanced binary trees,
the former can be seen either as a list of perfectly balance binary trees. A Merkle Mountain Range (MMR) is strictly
append-only: elements are added from the left to the right, adding a parent as soon as two children exist, filling up
the range accordingly.

The diagram below shows an MMR with 11 leaves and a total of 19 nodes, where each node is annotated with its order of
insertion:
```
Height

3              14
             /    \
            /      \
           /        \
          /          \
2        6            13
       /   \        /    \
1     2     5      9     12     17
     / \   / \    / \   /  \   /  \
0   0   1 3   4  7   8 10  11 15  16 18
```
This can be represented as a flat list, here storing the height of each node at their position of insertion:
```
0  1  2  3  4  5  6  7  8  9 10 11 12 13 14 15 16 17 18
0  0  1  0  0  1  2  0  0  1  0  0  1  2  3  0  0  1  0
```
The structure can be fully described simply from its size (19). It's also fairly simple, using fast binary operations,
to navigate within an MMR. Given a node's position _n_, we can compute its height, the position of its parent, its
siblings, etc.

## Hashing and Bagging
Just like with Merkle trees, parent nodes in an MMR have has their value the hash of their two children. We always
prepend the node's position in the MMR before hashing to avoid collisions. So for a leaf _l_ at index _n_ storing data
_D_, we have:
```
Node(l) = Hash(n | D)
```
And for any parent _p_ at index _m_:
```
Node(p) = Hash(m | Node(left_child(p)) | Node(right_child(p)))
```
Contrarily to a Merkle tree, an MMR generally has no single root by construction, so we need a method to compute one
(otherwise it would defeat the purpose of using a hash tree). This process is called _"bagging the peaks"_.

First, we identify the peaks of the MMR; we will define one method of doing so here. We first write another small
example MMR but with the indexes written as binary (instead of decimal), starting from 1:
```
Height

2        111
       /     \
1     11     110       1010
     /  \    / \      /    \
0   1   10 100 101  1000  1001  1011
```
This MMR has 11 nodes and its peaks are at position 111 (7), 1010 (10) and 1011 (11). We first notice how the first
leftmost peak is always going to be the highest and always _"all ones"_ when expressed in binary. Therefore, that peak
will have a position of the form _2^n - 1_ and will always be the largest such position that is inside the MMR
(its position is lesser than or equal to the total size). We process iteratively for an MMR of size 11:
```
2^0 - 1 = 0, and 0 <= 11
2^1 - 1 = 1, and 1 <= 11
2^2 - 1 = 3, and 3 <= 11
2^3 - 1 = 7, and 7 <= 11
2^4 - 1 = 15, and 15 > 11
```
Therefore, the first peak is 7. To find the next peak, we then need to _"jump"_ to its right sibling. If that node is not
in the MMR (and it won't), take its left child. If that child is not in the MMR either, keep taking its left child until
we have a node that exists in our MMR. Once we find that next peak, keep repeating the process until we're at the last
node.

All of these operations are very simple. Jumping to the right sibling of a node at height _h_ is simply adding
_2^(h+1) - 1_ to its position. Taking its left child is subtracting _2^h_.

Finally, once all the positions of the peaks are known, _"bagging"_ the peaks consists of hashing them iteratively from
the right, using the total size of the MMR as prefix. For an MMR of size _N_, 3 peaks _p1_, _p2_ and _p3_, each one
respectively having _n1_, _n2_ and _n3_ leaves, we get the final top peak:
```
P = Hash(N | Node(p3) | Hash(n1 + n2 | Node(p2) | Node(p1)))
```

### License

Licensed under Apache License, Version 2.0, (http://www.apache.org/licenses/LICENSE-2.0).

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be licensed as above, without any additional terms or
conditions.

### Sources

This text was mostly copied from the Grin documentation.