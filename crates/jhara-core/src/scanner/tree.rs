use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ustr::Ustr;

use crate::scanner::types::{NodeKind, ScanNode};

/// A node in the scan tree.
///
/// Stored in a flat `Vec<TreeNode>` rather than as heap-allocated linked
/// nodes. The parent–child relationship is encoded via `parent_idx`.
/// Because `jwalk` emits entries in pre-order (parents before children),
/// entries are naturally appended in topological order, which makes the
/// O(N) bottom-up size rollup trivial: iterate in reverse.
///
/// Path segments are interned via `ustr::Ustr`, a lock-free global cache.
/// For a 1M-file tree the interning reduces the backing string storage from
/// ~250 MB (one `String` per full path) to ~18 MB (one `Ustr` per unique
/// path segment, shared across all entries with the same name).
#[derive(Debug)]
pub struct TreeNode {
    /// Interned name of this entry (last path component).
    pub name: Ustr,

    /// Index of this entry's parent in `ScanTree::nodes`.
    /// `None` only for the root node(s).
    pub parent_idx: Option<usize>,

    /// Physical bytes allocated for this entry (from `ScanNode::physical_size`).
    /// For directories, this is the sum of all descendant physical sizes
    /// after `rollup()` is called.
    pub physical_size: u64,

    /// Logical bytes for this entry.
    /// For directories, this is the sum of all descendant logical sizes
    /// after `rollup()` is called.
    pub logical_size: u64,

    /// Modification time (seconds since Unix epoch).
    pub modification_secs: i64,

    /// Number of direct children (files and directories).
    pub child_count: u32,

    /// Entry classification.
    pub kind: NodeKind,

    /// Full path — stored as a PathBuf for tree query functions.
    /// This is the only non-interned per-node allocation.
    pub path: PathBuf,
}

/// The in-memory representation of a filesystem scan result.
///
/// ## Memory Layout
///
/// Nodes are stored in a flat `Vec<TreeNode>` in pre-order: a parent node
/// always appears before its children. This invariant is guaranteed by the
/// `jwalk` walker which emits entries top-down.
///
/// The `path_index` provides O(1) lookup from a `PathBuf` to its node index.
///
/// ## Size Rollup
///
/// Physical and logical sizes start as the entry's own size (zero for
/// directories). `rollup()` propagates children's sizes up to their parents
/// in a single O(N) reverse pass. Callers must invoke `rollup()` after
/// inserting all nodes from a completed scan.
///
/// ## Incremental Updates
///
/// For FSEvents-driven re-scans, `invalidate_subtree()` removes a subtree
/// from the tree and `path_index` so the new scan results can be re-inserted.
pub struct ScanTree {
    /// All nodes in pre-order. Index 0 is the conceptual root (may be a
    /// synthetic root if multiple scan roots were provided).
    pub nodes: Vec<TreeNode>,

    /// Maps each node's full path to its index in `nodes`.
    path_index: HashMap<PathBuf, usize>,
}

impl ScanTree {
    /// Create an empty tree with pre-allocated capacity.
    ///
    /// `capacity` should be set to the expected number of filesystem entries
    /// to avoid reallocations. For a typical developer home directory,
    /// 500 000 is a reasonable starting point.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            nodes: Vec::with_capacity(capacity),
            path_index: HashMap::with_capacity(capacity),
        }
    }

    /// Insert a `ScanNode` into the tree.
    ///
    /// Finds the parent by looking up `node.path.parent()` in `path_index`.
    /// If the parent is not yet in the tree (can happen if the root node itself
    /// is the first entry), the node is treated as a root and `parent_idx` is `None`.
    ///
    /// This is called for every entry in the scan, typically millions of times.
    /// It must stay fast: the only allocations are the `PathBuf` stored in the
    /// node and the `HashMap` entry.
    pub fn insert(&mut self, node: ScanNode) {
        let parent_idx = node
            .path
            .parent()
            .and_then(|p| self.path_index.get(p))
            .copied();

        // Increment the parent's child count
        if let Some(pidx) = parent_idx {
            self.nodes[pidx].child_count += 1;
        }

        let idx = self.nodes.len();
        self.path_index.insert(node.path.clone(), idx);

        self.nodes.push(TreeNode {
            name: Ustr::from(&node.name),
            parent_idx,
            physical_size: node.physical_size,
            logical_size: node.logical_size,
            modification_secs: node.modification_secs,
            child_count: 0,
            kind: node.kind,
            path: node.path,
        });
    }

    /// Insert a batch of `ScanNode`s, as delivered by the scanner callback.
    pub fn insert_batch(&mut self, batch: Vec<ScanNode>) {
        // Reserve capacity for the incoming batch to avoid per-insert reallocations
        self.nodes.reserve(batch.len());
        self.path_index.reserve(batch.len());
        for node in batch {
            self.insert(node);
        }
    }

    /// Propagate file sizes up the tree from leaves to roots.
    ///
    /// Must be called once after all nodes have been inserted.
    /// Calling it a second time without clearing will double-count sizes.
    ///
    /// ## Algorithm
    ///
    /// Because nodes are stored in topological (pre-)order, iterating in
    /// *reverse* guarantees that when we process node `i`, all its children
    /// (which have indices > i) have already had their sizes propagated up
    /// to *their* parents. One pass, O(N), no recursion, no locking.
    ///
    /// This replaces the Swift `propagateUp()` that was called on every
    /// insertion (O(N × depth)), which caused contention at the parent-node
    /// level during concurrent scans.
    pub fn rollup(&mut self) {
        for i in (1..self.nodes.len()).rev() {
            let phys = self.nodes[i].physical_size;
            let log = self.nodes[i].logical_size;
            if let Some(parent_idx) = self.nodes[i].parent_idx {
                self.nodes[parent_idx].physical_size += phys;
                self.nodes[parent_idx].logical_size += log;
            }
        }
    }

    /// Return the total physical size of the subtree rooted at `path`.
    ///
    /// Must be called *after* `rollup()`. Returns `None` if `path` is not
    /// in the tree.
    pub fn physical_size(&self, path: &Path) -> Option<u64> {
        self.path_index
            .get(path)
            .map(|&idx| self.nodes[idx].physical_size)
    }

    /// Return the total logical size of the subtree rooted at `path`.
    pub fn logical_size(&self, path: &Path) -> Option<u64> {
        self.path_index
            .get(path)
            .map(|&idx| self.nodes[idx].logical_size)
    }

    /// Return the node at `path`, or `None` if not present.
    pub fn node(&self, path: &Path) -> Option<&TreeNode> {
        self.path_index.get(path).map(|&idx| &self.nodes[idx])
    }

    /// Iterate over the direct children of `path`.
    pub fn children(&self, path: &Path) -> impl Iterator<Item = &TreeNode> {
        let parent_idx = self.path_index.get(path).copied();
        self.nodes
            .iter()
            .filter(move |n| n.parent_idx == parent_idx && parent_idx.is_some())
    }

    /// Remove an entire subtree rooted at `path` from the tree.
    ///
    /// Used by the FSEvents incremental re-scan: when a directory changes,
    /// its subtree is invalidated and the scanner re-populates it.
    ///
    /// This is O(N) over the total number of nodes in the tree — acceptable
    /// for incremental updates where the tree is typically already built.
    /// If the subtree covers millions of nodes, consider a full re-scan instead.
    pub fn invalidate_subtree(&mut self, path: &Path) {
        let Some(&root_idx) = self.path_index.get(path) else {
            return;
        };

        // Collect all indices that are descendants of root_idx.
        // A node at index j is a descendant if, following parent_idx links,
        // we eventually reach root_idx.
        let is_descendant: Vec<bool> = {
            let n = self.nodes.len();
            let mut desc = vec![false; n];
            desc[root_idx] = true;

            // Forward pass: a node is a descendant if its parent is.
            // This works because pre-order guarantees parent_idx < child_idx.
            for i in (root_idx + 1)..n {
                if let Some(p) = self.nodes[i].parent_idx {
                    if desc[p] {
                        desc[i] = true;
                    }
                }
            }
            desc
        };

        // Remove from path_index
        self.path_index.retain(|_, &mut idx| !is_descendant[idx]);

        // Mark nodes for removal by setting kind to a sentinel — we cannot
        // remove from the Vec mid-iteration without invalidating indices.
        // Rebuild the Vec and path_index from the survivors.
        let survivors: Vec<TreeNode> = self
            .nodes
            .drain(..)
            .enumerate()
            .filter(|(i, _)| !is_descendant[*i])
            .map(|(_, n)| n)
            .collect();

        self.nodes = survivors;

        // Rebuild path_index with corrected indices after the compaction.
        self.path_index.clear();
        for (idx, node) in self.nodes.iter().enumerate() {
            self.path_index.insert(node.path.clone(), idx);
        }
    }

    /// Return the number of nodes currently in the tree.
    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Approximate heap bytes used by this tree.
    ///
    /// Useful for memory profiling. Does not account for `ustr`'s global
    /// intern table (which is shared across all `ScanTree` instances).
    pub fn approximate_heap_bytes(&self) -> usize {
        let nodes_bytes = self.nodes.len() * std::mem::size_of::<TreeNode>();
        // HashMap overhead: roughly 2× the number of entries
        let index_bytes = self.path_index.len()
            * (std::mem::size_of::<PathBuf>() + std::mem::size_of::<usize>())
            * 2;
        nodes_bytes + index_bytes
    }
}

impl Default for ScanTree {
    fn default() -> Self {
        Self::with_capacity(64_000)
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(path: &str, size: u64, kind: NodeKind) -> ScanNode {
        let p = PathBuf::from(path);
        ScanNode {
            name: p.file_name().unwrap().to_string_lossy().into_owned(),
            path: p,
            inode: 0,
            device_id: 0,
            physical_size: size,
            logical_size: size,
            modification_secs: 0,
            modification_nanos: 0,
            link_count: 1,
            kind,
        }
    }

    #[test]
    fn insert_and_lookup() {
        let mut tree = ScanTree::default();
        tree.insert(make_node("/project", 0, NodeKind::DirPre));
        tree.insert(make_node("/project/src", 0, NodeKind::DirPre));
        tree.insert(make_node("/project/src/main.rs", 1024, NodeKind::File));

        assert!(tree.node(Path::new("/project")).is_some());
        assert!(tree.node(Path::new("/project/src/main.rs")).is_some());
        assert_eq!(tree.len(), 3);
    }

    #[test]
    fn rollup_propagates_sizes_to_root() {
        let mut tree = ScanTree::default();
        // /project (dir)
        //   /project/target (dir)
        //     /project/target/binary (file, 1 MB)
        //   /project/src (dir)
        //     /project/src/main.rs (file, 4 KB)
        tree.insert(make_node("/project", 0, NodeKind::DirPre));
        tree.insert(make_node("/project/target", 0, NodeKind::DirPre));
        tree.insert(make_node(
            "/project/target/binary",
            1_048_576,
            NodeKind::File,
        ));
        tree.insert(make_node("/project/src", 0, NodeKind::DirPre));
        tree.insert(make_node("/project/src/main.rs", 4_096, NodeKind::File));
        tree.rollup();

        let root_phys = tree.physical_size(Path::new("/project")).unwrap();
        assert_eq!(root_phys, 1_048_576 + 4_096);

        let target_phys = tree.physical_size(Path::new("/project/target")).unwrap();
        assert_eq!(target_phys, 1_048_576);
    }

    #[test]
    fn rollup_does_not_double_count() {
        let mut tree = ScanTree::default();
        tree.insert(make_node("/a", 0, NodeKind::DirPre));
        tree.insert(make_node("/a/b", 0, NodeKind::DirPre));
        tree.insert(make_node("/a/b/f1", 100, NodeKind::File));
        tree.insert(make_node("/a/b/f2", 200, NodeKind::File));
        tree.rollup();

        // /a/b should be 300, /a should also be 300
        assert_eq!(tree.physical_size(Path::new("/a/b")).unwrap(), 300);
        assert_eq!(tree.physical_size(Path::new("/a")).unwrap(), 300);
    }

    #[test]
    fn physical_size_returns_none_for_unknown_path() {
        let tree = ScanTree::default();
        assert!(tree.physical_size(Path::new("/nonexistent")).is_none());
    }

    #[test]
    fn invalidate_subtree_removes_descendants() {
        let mut tree = ScanTree::default();
        tree.insert(make_node("/root", 0, NodeKind::DirPre));
        tree.insert(make_node("/root/keep", 0, NodeKind::DirPre));
        tree.insert(make_node("/root/keep/file.txt", 100, NodeKind::File));
        tree.insert(make_node("/root/remove", 0, NodeKind::DirPre));
        tree.insert(make_node("/root/remove/stale", 5000, NodeKind::File));

        tree.invalidate_subtree(Path::new("/root/remove"));

        assert!(tree.node(Path::new("/root/remove")).is_none());
        assert!(tree.node(Path::new("/root/remove/stale")).is_none());
        // Unaffected subtree must survive
        assert!(tree.node(Path::new("/root/keep/file.txt")).is_some());
    }

    #[test]
    fn approximate_heap_bytes_is_nonzero_after_insert() {
        let mut tree = ScanTree::default();
        tree.insert(make_node("/x", 0, NodeKind::DirPre));
        assert!(tree.approximate_heap_bytes() > 0);
    }

    #[test]
    fn insert_batch_equivalent_to_individual_inserts() {
        let nodes_a = vec![
            make_node("/p", 0, NodeKind::DirPre),
            make_node("/p/f.txt", 512, NodeKind::File),
        ];
        let nodes_b = nodes_a.clone();

        let mut tree_a = ScanTree::default();
        for n in nodes_a {
            tree_a.insert(n);
        }
        tree_a.rollup();

        let mut tree_b = ScanTree::default();
        tree_b.insert_batch(nodes_b);
        tree_b.rollup();

        assert_eq!(
            tree_a.physical_size(Path::new("/p")),
            tree_b.physical_size(Path::new("/p")),
        );
    }
}
