//! The order of the iteration of the sets of the reference ✓.
//!
//! Java 8's `HashMap` ✓ — the table of the buckets ✓:
//!
//! * the table has sixteen buckets at the start ✓ and it is doubled when the
//!   size of the table passes the threshold of three quarters ✓;
//! * the bucket of a key is `(hash ^ (hash >>> 16)) & (capacity - 1)` ✓
//!   (`HashMap.hash(Object)` ✓ — the spread of the hash ✓);
//! * the keys of a bucket are in the order of the insertion ✓ — until the
//!   bucket reaches eight keys ✓: then the bucket becomes a tree ✓ (if the
//!   table has at least sixty-four buckets ✓) and the order inside it is the
//!   order of the chain of the tree ✓ (`HashMap.TreeNode.next` ✓);
//! * the growth of the table splits every bucket in two ✓ and keeps the order
//!   inside the buckets ✓ (`HashMap.TreeNode.split` ✓).
//!
//! The port iterates its own sets ✓ (a `HashSet` of Rust ✓ — the order is
//! random for every process ✓) ⇒ the order of the iteration of the reference
//! has to be reproduced ✓ (see W-53 of the plan ✓).
//!
//! The emulation follows the code of the JDK 8 ✓ (`java/util/HashMap.java` ✓
//! — `putVal` ✓, `resize` ✓, `treeifyBin` ✓, `TreeNode.treeify` ✓,
//! `TreeNode.putTreeVal` ✓, `TreeNode.balanceInsertion` ✓,
//! `TreeNode.split` ✓ and `TreeNode.moveRootToFront` ✓).

use std::collections::HashSet;
use std::hash::Hash;

/// The number of the keys of a bucket that turns it into a tree ✓
/// (`HashMap.TREEIFY_THRESHOLD` ✓).
const TREEIFY_THRESHOLD: usize = 8;
/// The number of the buckets that a tree needs ✓
/// (`HashMap.MIN_TREEIFY_CAPACITY` ✓).
const MIN_TREEIFY_CAPACITY: usize = 64;
/// The number of the keys of a bucket that turns it back into a list ✓
/// (`HashMap.UNTREEIFY_THRESHOLD` ✓).
const UNTREEIFY_THRESHOLD: usize = 6;

/// The spread of the hash of Java ✓ (`HashMap.hash(Object)` ✓).
fn spread_hash(hash: i32) -> u32 {
    let h = hash as u32;
    h ^ (h >> 16)
}

/// A key of the table ✓.
#[derive(Clone, Debug)]
struct Node {
    /// The spread of the hash of the key ✓ (`HashMap.Node.hash` ✓).
    hash: u32,
    /// The index of the key ✓.
    key: usize,
    /// The links of the red-black tree ✓ (`HashMap.TreeNode` ✓).
    parent: Option<usize>,
    left: Option<usize>,
    right: Option<usize>,
    red: bool,
    /// The chain of the bucket ✓ (`HashMap.Node.next` ✓).
    next: Option<usize>,
}

/// A bucket of the table ✓.
#[derive(Clone, Debug, Default)]
struct Bin {
    /// The first key of the chain ✓.
    head: Option<usize>,
    /// The last key of the chain ✓.
    tail: Option<usize>,
    /// The root of the tree of the bucket ✓ (only for a tree ✓).
    root: Option<usize>,
    /// The bucket is a tree ✓.
    tree: bool,
    /// The number of the keys of the bucket ✓.
    len: usize,
}

/// The table of Java ✓ (`HashMap` ✓).
#[derive(Clone, Debug, Default)]
struct Table {
    /// The keys of the table ✓.
    nodes: Vec<Node>,
    /// The buckets of the table ✓.
    bins: Vec<Bin>,
    /// The number of the buckets ✓.
    capacity: usize,
    /// The number of the keys that turns the table ✓.
    threshold: usize,
    /// The number of the keys of the table ✓.
    size: usize,
}

impl Table {
    /// Adds a key to the table ✓ (the hash is the spread hash of Java ✓).
    fn put(&mut self, hash: u32, key: usize) {
        if self.capacity == 0 {
            // Java `HashMap.putVal` ✓: the table is allocated with the first
            // key ✓ — sixteen buckets ✓.
            self.capacity = 16;
            self.threshold = 12;
            self.bins = vec![Bin::default(); self.capacity];
        }
        let index = (hash as usize) & (self.capacity - 1);
        if self.bins[index].tree {
            self.put_tree_val(index, hash, key);
        } else {
            // Java `HashMap.putVal` ✓: the key is added at the end of the
            // chain of the bucket ✓.
            let node = self.new_node(hash, key, None);
            let tail = self.bins[index].tail;
            match tail {
                Some(tail) => self.nodes[tail].next = Some(node),
                None => self.bins[index].head = Some(node),
            }
            self.bins[index].tail = Some(node);
            self.bins[index].len += 1;
            if self.bins[index].len >= TREEIFY_THRESHOLD {
                // Java `HashMap.putVal` ✓: the bucket of eight keys turns into
                // a tree ✓.
                self.treeify_bin(index);
            }
        }
        self.size += 1;
        if self.size > self.threshold {
            // Java `HashMap.putVal` ✓: the table grows when the size passes
            // the threshold ✓.
            self.resize();
        }
    }

    /// Creates a key of the table ✓.
    fn new_node(&mut self, hash: u32, key: usize, next: Option<usize>) -> usize {
        self.nodes.push(Node {
            hash,
            key,
            parent: None,
            left: None,
            right: None,
            red: false,
            next,
        });
        self.nodes.len() - 1
    }

    /// The order of the iteration of the table ✓.
    fn order(&self) -> Vec<usize> {
        let mut order = Vec::with_capacity(self.size);
        for bin in &self.bins {
            let mut node = bin.head;
            while let Some(current) = node {
                order.push(self.nodes[current].key);
                node = self.nodes[current].next;
            }
        }
        order
    }

    /// Java `HashMap.treeifyBin` ✓.
    fn treeify_bin(&mut self, index: usize) {
        if self.capacity < MIN_TREEIFY_CAPACITY {
            // Java `HashMap.treeifyBin` ✓: a small table grows instead of the
            // tree ✓.
            self.resize();
            return;
        }
        // The chain of the bucket is rebuilt ✓ (the order is kept ✓) and the
        // tree is built from it ✓.
        self.treeify(index);
    }

    /// Java `HashMap.TreeNode.treeify` ✓.
    fn treeify(&mut self, index: usize) {
        let mut root: Option<usize> = None;
        let mut node = self.bins[index].head;
        while let Some(current) = node {
            let next = self.nodes[current].next;
            self.nodes[current].left = None;
            self.nodes[current].right = None;
            match root {
                None => {
                    self.nodes[current].parent = None;
                    self.nodes[current].red = false;
                    root = Some(current);
                }
                Some(mut parent) => loop {
                    let hash = self.nodes[current].hash;
                    // The comparison of the tree ✓: the hashes are compared as
                    // the signed integers of Java ✓.
                    let direction = match (self.nodes[parent].hash as i32).cmp(&(hash as i32)) {
                        std::cmp::Ordering::Greater => -1,
                        std::cmp::Ordering::Less => 1,
                        std::cmp::Ordering::Equal => {
                            // The tie-break of the reference ✓ (`tieBreakOrder`
                            // ✓) is the identity of the key ✓ — it is not
                            // reproducible ✓ ⇒ the first key wins ✓.
                            1
                        }
                    };
                    let child = if direction <= 0 {
                        self.nodes[parent].left
                    } else {
                        self.nodes[parent].right
                    };
                    match child {
                        None => {
                            self.nodes[current].parent = Some(parent);
                            if direction <= 0 {
                                self.nodes[parent].left = Some(current);
                            } else {
                                self.nodes[parent].right = Some(current);
                            }
                            root = Some(self.balance_insertion(root.unwrap(), current));
                            break;
                        }
                        Some(child) => parent = child,
                    }
                },
            }
            node = next;
        }
        self.bins[index].root = root;
        self.bins[index].tree = true;
        // Java `HashMap.TreeNode.moveRootToFront` ✓: the root is the first
        // key of the chain ✓.
        self.move_root_to_front(index, root);
    }

    /// Java `HashMap.TreeNode.moveRootToFront` ✓.
    fn move_root_to_front(&mut self, index: usize, root: Option<usize>) {
        let Some(root) = root else { return };
        let Some(first) = self.bins[index].head else {
            return;
        };
        if root == first {
            return;
        }
        // The root is taken out of the chain ✓.
        let mut previous = None;
        let mut current = self.bins[index].head;
        while let Some(node) = current {
            if node == root {
                break;
            }
            previous = Some(node);
            current = self.nodes[node].next;
        }
        let after = self.nodes[root].next;
        if let Some(previous) = previous {
            self.nodes[previous].next = after;
        }
        // The root is the first key of the chain ✓.
        self.nodes[root].next = self.bins[index].head;
        self.bins[index].head = Some(root);
        if after.is_none() {
            self.bins[index].tail = Some(root);
        }
    }

    /// Java `HashMap.TreeNode.putTreeVal` ✓.
    fn put_tree_val(&mut self, index: usize, hash: u32, key: usize) {
        let Some(root) = self.bins[index].root else {
            return;
        };
        let mut parent = root;
        loop {
            // The comparison of the tree ✓ (the signed hashes ✓).
            let direction = match (self.nodes[parent].hash as i32).cmp(&(hash as i32)) {
                std::cmp::Ordering::Greater => -1,
                std::cmp::Ordering::Less => 1,
                std::cmp::Ordering::Equal => 1,
            };
            let child = if direction <= 0 {
                self.nodes[parent].left
            } else {
                self.nodes[parent].right
            };
            match child {
                None => {
                    // The key is added after its parent in the chain ✓.
                    let after = self.nodes[parent].next;
                    let node = self.new_node(hash, key, after);
                    if direction <= 0 {
                        self.nodes[parent].left = Some(node);
                    } else {
                        self.nodes[parent].right = Some(node);
                    }
                    self.nodes[parent].next = Some(node);
                    self.nodes[node].parent = Some(parent);
                    if after.is_none() {
                        self.bins[index].tail = Some(node);
                    }
                    self.bins[index].len += 1;
                    let root = self.balance_insertion(root, node);
                    self.bins[index].root = Some(root);
                    self.move_root_to_front(index, Some(root));
                    return;
                }
                Some(child) => parent = child,
            }
        }
    }

    /// Java `HashMap.TreeNode.balanceInsertion` ✓.
    fn balance_insertion(&mut self, mut root: usize, mut x: usize) -> usize {
        self.nodes[x].red = true;
        loop {
            let Some(xp) = self.nodes[x].parent else {
                // The node is the root of the tree ✓.
                self.nodes[x].red = false;
                return x;
            };
            if !self.nodes[xp].red {
                return root;
            }
            let Some(xpp) = self.nodes[xp].parent else {
                return root;
            };
            if self.nodes[xpp].left == Some(xp) {
                // The uncle is the right child of the grandfather ✓.
                if let Some(uncle) = self.nodes[xpp].right {
                    if self.nodes[uncle].red {
                        // The red-red case ✓: the colours are pushed up ✓.
                        self.nodes[uncle].red = false;
                        self.nodes[xp].red = false;
                        self.nodes[xpp].red = true;
                        x = xpp;
                        continue;
                    }
                }
                // The triangle case ✓.
                let mut xp = Some(xp);
                let mut xpp = Some(xpp);
                if self.nodes[xp.unwrap()].right == Some(x) {
                    x = xp.unwrap();
                    root = self.rotate_left(root, x);
                    xp = self.nodes[x].parent;
                    xpp = xp.and_then(|parent| self.nodes[parent].parent);
                }
                if let Some(xp) = xp {
                    self.nodes[xp].red = false;
                    if let Some(xpp) = xpp {
                        self.nodes[xpp].red = true;
                        root = self.rotate_right(root, xpp);
                    }
                }
            } else {
                // The uncle is the left child of the grandfather ✓.
                if let Some(uncle) = self.nodes[xpp].left {
                    if self.nodes[uncle].red {
                        self.nodes[uncle].red = false;
                        self.nodes[xp].red = false;
                        self.nodes[xpp].red = true;
                        x = xpp;
                        continue;
                    }
                }
                // The triangle case ✓.
                let mut xp = Some(xp);
                let mut xpp = Some(xpp);
                if self.nodes[xp.unwrap()].left == Some(x) {
                    x = xp.unwrap();
                    root = self.rotate_right(root, x);
                    xp = self.nodes[x].parent;
                    xpp = xp.and_then(|parent| self.nodes[parent].parent);
                }
                if let Some(xp) = xp {
                    self.nodes[xp].red = false;
                    if let Some(xpp) = xpp {
                        self.nodes[xpp].red = true;
                        root = self.rotate_left(root, xpp);
                    }
                }
            }
        }
    }

    /// Java `HashMap.TreeNode.rotateRight` ✓.
    fn rotate_right(&mut self, root: usize, p: usize) -> usize {
        let mut root = root;
        let Some(l) = self.nodes[p].left else {
            return root;
        };
        let lr = self.nodes[l].right;
        self.nodes[p].left = lr;
        if let Some(lr) = lr {
            self.nodes[lr].parent = Some(p);
        }
        let pp = self.nodes[p].parent;
        self.nodes[l].parent = pp;
        match pp {
            None => {
                root = l;
                self.nodes[root].red = false;
            }
            Some(pp) => {
                if self.nodes[pp].right == Some(p) {
                    self.nodes[pp].right = Some(l);
                } else {
                    self.nodes[pp].left = Some(l);
                }
            }
        }
        self.nodes[l].right = Some(p);
        self.nodes[p].parent = Some(l);
        root
    }

    /// Java `HashMap.TreeNode.rotateLeft` ✓.
    fn rotate_left(&mut self, root: usize, p: usize) -> usize {
        let mut root = root;
        let Some(r) = self.nodes[p].right else {
            return root;
        };
        let rl = self.nodes[r].left;
        self.nodes[p].right = rl;
        if let Some(rl) = rl {
            self.nodes[rl].parent = Some(p);
        }
        let pp = self.nodes[p].parent;
        self.nodes[r].parent = pp;
        match pp {
            None => {
                root = r;
                self.nodes[root].red = false;
            }
            Some(pp) => {
                if self.nodes[pp].left == Some(p) {
                    self.nodes[pp].left = Some(r);
                } else {
                    self.nodes[pp].right = Some(r);
                }
            }
        }
        self.nodes[r].left = Some(p);
        self.nodes[p].parent = Some(r);
        root
    }

    /// Java `HashMap.resize` ✓.
    fn resize(&mut self) {
        let old_capacity = self.capacity;
        let new_capacity = if old_capacity == 0 {
            16
        } else {
            old_capacity * 2
        };
        self.threshold = new_capacity * 3 / 4;
        let mut new_bins = vec![Bin::default(); new_capacity];
        for index in 0..old_capacity {
            let bin = self.bins[index].clone();
            let Some(head) = bin.head else { continue };
            if bin.tree {
                // Java `HashMap.TreeNode.split` ✓: the chain of the tree is
                // split in two ✓ and the order is kept ✓.
                let (lo, hi) = self.split_tree(&bin, old_capacity);
                new_bins[index] = lo;
                new_bins[index + old_capacity] = hi;
            } else {
                // Java `HashMap.resize` ✓: the chain of the bucket is split in
                // two ✓ and the order is kept ✓.
                let mut node = Some(head);
                while let Some(current) = node {
                    node = self.nodes[current].next;
                    let target = if self.nodes[current].hash as usize & old_capacity == 0 {
                        index
                    } else {
                        index + old_capacity
                    };
                    if new_bins[target].tail.is_none() {
                        new_bins[target].head = Some(current);
                    } else {
                        let tail = new_bins[target].tail.unwrap();
                        self.nodes[tail].next = Some(current);
                    }
                    new_bins[target].tail = Some(current);
                    new_bins[target].len += 1;
                }
                // The end of the chain of every part ✓.
                for target in [index, index + old_capacity] {
                    if let Some(tail) = new_bins[target].tail {
                        self.nodes[tail].next = None;
                    }
                }
            }
        }
        self.bins = new_bins;
        self.capacity = new_capacity;
        // The trees of the new buckets are built ✓ (`TreeNode.split` ✓).
        let trees: Vec<usize> = (0..self.bins.len())
            .filter(|&index| self.bins[index].tree)
            .collect();
        for index in trees {
            self.treeify(index);
        }
    }

    /// Java `HashMap.TreeNode.split` ✓.
    ///
    /// The chain of the tree is split in two ✓ — the order is kept ✓ — and
    /// every part is a list again ✓ when it is short enough ✓
    /// (`UNTREEIFY_THRESHOLD` ✓).
    fn split_tree(&mut self, bin: &Bin, capacity: usize) -> (Bin, Bin) {
        let mut lo = Bin::default();
        let mut hi = Bin::default();
        let mut node = bin.head;
        while let Some(current) = node {
            node = self.nodes[current].next;
            self.nodes[current].next = None;
            let target = if self.nodes[current].hash as usize & capacity == 0 {
                &mut lo
            } else {
                &mut hi
            };
            if let Some(tail) = target.tail {
                self.nodes[tail].next = Some(current);
            } else {
                target.head = Some(current);
            }
            target.tail = Some(current);
            target.len += 1;
        }
        // A short part is a list again ✓.
        if lo.len <= UNTREEIFY_THRESHOLD {
            lo.tree = false;
            lo.root = None;
        }
        if hi.len <= UNTREEIFY_THRESHOLD {
            hi.tree = false;
            hi.root = None;
        }
        (lo, hi)
    }
}

/// The order of the iteration of a `java.util.HashSet` ✓.
///
/// The keys are given in the order of the insertion ✓ with the hash of Java
/// ✓ (`hashCode()` ✓); the result holds the indices of the keys in the order
/// of the iteration of the reference ✓.
pub fn java_hash_set_order(hashes: &[i32]) -> Vec<usize> {
    let mut table = Table::default();
    for (index, &hash) in hashes.iter().enumerate() {
        table.put(spread_hash(hash), index);
    }
    table.order()
}

/// The set of the reference ✓ (`java.util.HashSet` ✓) with the order of the
/// iteration of Java ✓.
///
/// The hash of every key is given by the caller ✓ (`hashCode()` ✓ of the
/// reference ✓) — the duplicates are ignored ✓ (as in Java ✓).
#[derive(Clone, Debug)]
pub struct JavaHashSet<T> {
    /// The keys of the set ✓ with their hash ✓ — in the order of the
    /// insertion ✓.
    entries: Vec<(i32, T)>,
    /// The keys that are in the set ✓ (the duplicates are ignored ✓).
    keys: HashSet<T>,
}

impl<T: Copy + Eq + Hash> Default for JavaHashSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy + Eq + Hash> JavaHashSet<T> {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            keys: HashSet::new(),
        }
    }

    /// Adds a key to the set ✓ — the hash is the hash of Java ✓
    /// (`hashCode()` ✓).
    ///
    /// The returns `true` when the key was not in the set ✓ (as
    /// `HashSet.add` ✓).
    pub fn insert(&mut self, hash: i32, key: T) -> bool {
        if !self.keys.insert(key) {
            return false;
        }
        self.entries.push((hash, key));
        true
    }

    /// Removes every key of the set ✓ (`HashSet.clear` ✓).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.keys.clear();
    }

    /// The number of the keys of the set ✓ (`HashSet.size` ✓).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The set is empty ✓ (`HashSet.isEmpty` ✓).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The keys of the set in the order of the iteration of Java ✓.
    pub fn iteration_order(&self) -> Vec<T> {
        let hashes: Vec<i32> = self.entries.iter().map(|(hash, _)| *hash).collect();
        java_hash_set_order(&hashes)
            .into_iter()
            .map(|index| self.entries[index].1)
            .collect()
    }
}

/// The hash of the reference of a `ChunkPosition` ✓
/// (`ChunkPosition.hashCode()` ✓ — `x * 8976890 + y * 981131 + z` ✓).
pub fn chunk_position_hash(x: i32, y: i32, z: i32) -> i32 {
    x.wrapping_mul(8_976_890)
        .wrapping_add(y.wrapping_mul(981_131))
        .wrapping_add(z)
}

/// The hash of the reference of a `ChunkCoordIntPair` ✓
/// (`ChunkCoordIntPair.hashCode()` ✓ — `ChunkCoordIntPair.a(int, int)` ✓).
pub fn chunk_coord_hash(x: i32, z: i32) -> i32 {
    let high = if x < 0 { i32::MIN } else { 0 };
    let low = if z < 0 { 0x8000 } else { 0 };
    high | ((x & 32767) << 16) | low | (z & 32767)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The order of the iteration of the sets of the reference ✓.
    ///
    /// The golden file `tests/golden/poseidon_hash_order.txt` ✓ holds the
    /// order of the reference ✓ (`tools/poseidon_hash/PoseidonHashOrder.java`
    /// ✓).
    #[test]
    fn the_order_of_the_sets_of_the_reference_matches() {
        let golden = include_str!("../../tests/golden/poseidon_hash_order.txt");
        let mut scenarios: Vec<(String, Vec<String>)> = Vec::new();
        for line in golden.lines() {
            if let Some(rest) = line.strip_prefix("# scenario=") {
                scenarios.push((rest.to_string(), Vec::new()));
                continue;
            }
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            if let Some(last) = scenarios.last_mut() {
                last.1.push(line.to_string());
            }
        }
        for (name, wanted) in &scenarios {
            let actual: Vec<String> = match name.as_str() {
                "chunks" => {
                    // Java `World.j()` ✓: the chunks around the player ✓.
                    let mut set = JavaHashSet::new();
                    for k in -9..=9 {
                        for l in -9..=9 {
                            set.insert(chunk_coord_hash(k, l), (k, l));
                        }
                    }
                    set.iteration_order()
                        .into_iter()
                        .map(|(x, z)| format!("{x} {z}"))
                        .collect()
                }
                "wire" => {
                    // Java `BlockRedstoneWire.b` ✓: the seven positions around
                    // every wire that changes ✓.
                    let mut set = JavaHashSet::new();
                    for w in [(8, 64, 8), (9, 64, 8), (8, 64, 9), (10, 64, 8), (8, 64, 7)] {
                        for (dx, dy, dz) in [
                            (0, 0, 0),
                            (-1, 0, 0),
                            (1, 0, 0),
                            (0, -1, 0),
                            (0, 1, 0),
                            (0, 0, -1),
                            (0, 0, 1),
                        ] {
                            let (x, y, z) = (w.0 + dx, w.1 + dy, w.2 + dz);
                            set.insert(chunk_position_hash(x, y, z), (x, y, z));
                        }
                    }
                    set.iteration_order()
                        .into_iter()
                        .map(|(x, y, z)| format!("{x} {y} {z}"))
                        .collect()
                }
                other => {
                    if let Some(rest) = other.strip_prefix("random:") {
                        // The random keys ✓ — the stream of the reference ✓
                        // (`java.util.Random` ✓).
                        let (count, seed) = rest.split_once(':').expect("the scenario");
                        let count: i32 = count.parse().expect("the number of the keys");
                        let seed: i64 = seed.parse().expect("the seed");
                        let mut random = crate::util::rng::JavaRandom::new(seed);
                        let mut set = JavaHashSet::new();
                        for _ in 0..count {
                            let key = random.next_i32();
                            set.insert(key, key);
                        }
                        set.iteration_order()
                            .into_iter()
                            .map(|key| key.to_string())
                            .collect()
                    } else {
                        // The keys `0..n-1` ✓ and the keys `i * 64` ✓ — the
                        // hash of the `Integer` ✓ is its value ✓.
                        let (kind, count) = other.split_once(':').expect("the scenario");
                        let count: i32 = count.parse().expect("the number of the keys");
                        let mut set = JavaHashSet::new();
                        for i in 0..count {
                            let key = if kind == "collide" { i * 64 } else { i };
                            set.insert(key, key);
                        }
                        set.iteration_order()
                            .into_iter()
                            .map(|key| key.to_string())
                            .collect()
                    }
                }
            };
            assert_eq!(
                actual, *wanted,
                "the order of the set of the scenario {name} differs"
            );
        }
    }
}
