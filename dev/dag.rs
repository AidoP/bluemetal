use crate::tool::Tool;
use crate::JobPool;

const TRIE_FACTOR: usize = 4;
const _: () = assert!(TRIE_FACTOR.is_power_of_two());

#[derive(Debug)]
pub struct Target {
    /// Target Name. Typically, the primary output file name.
    pub name: &'static str,
    pub tool: &'static dyn Tool,
    pub depends: &'static [&'static str],
    /// The index of the child nodes, or `0` if there is no child.
    ///
    /// Logically, `0` cannot be a child as that would introduce a cycle as the
    /// root node is always item `0`.
    children: [usize; TRIE_FACTOR],
}
impl Target {
    pub const fn new(name: &'static str, tool: &'static dyn Tool) -> Self {
        assert!(!name.is_empty(), "target name cannot be empty");
        if let [b'/', ..] | [b'.', b'/', ..] | [b'.', b'.', b'/', ..] = name.as_bytes() {
            panic!("target name cannot start with `/`, `./` or `../`");
        }
        Self {
            name,
            tool,
            depends: &[],
            children: [0; TRIE_FACTOR],
        }
    }
    pub const fn depends(mut self, targets: &'static [&'static str]) -> Self {
        self.depends = targets;
        self
    }
    pub fn build(&self, pool: &mut JobPool) -> Result<(), ()> {
        self.tool.build(self, pool)
    }
}

const fn hash_str(s: &str) -> usize {
    let s = s.as_bytes();
    let mut hash = 0usize;
    let mut i = 0;
    while i < s.len() {
        hash = hash.wrapping_add(s[i] as usize);
        hash = hash.wrapping_add(hash << 10);
        hash ^= hash >> 6;
        i += 1;
    }

    hash = hash.wrapping_add(hash << 3);
    hash ^= hash >> 11;
    hash = hash.wrapping_add(hash << 15);
    hash
}

const fn str_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Returns the index of the item with the given name in the graph.
///
/// If `Err`, the index of the last node found, and the index of the child node
/// it would have continued searching through.
const fn find(graph: &[Target], name: &str) -> Result<usize, (usize, usize)> {
    let mut index = 0;
    let mut hash = hash_str(name);
    loop {
        let target = &graph[index];
        if str_eq(target.name, name) {
            return Ok(index);
        }
        let q = hash & (TRIE_FACTOR - 1);
        let next_index = target.children[q];
        if next_index == 0 {
            return Err((index, q));
        }
        index = next_index;

        hash = hash >> TRIE_FACTOR;
    }
}

#[repr(transparent)]
#[derive(Debug)]
pub struct Plan<T: AsRef<[Target]> + ?Sized = [Target]>(T);
impl<const N: usize> Plan<[Target; N]> {
    /// Compiles a build [`Plan`] as a hash table.
    pub const fn new(mut targets: [Target; N]) -> Self {
        // skip inserting the root node
        let mut i = 1;
        while i < targets.len() {
            let target = &targets[i];
            match find(&targets, target.name) {
                Ok(_) => panic!("duplicate item"),
                Err((end, q)) => {
                    targets[end].children[q] = i;
                }
            }
            i += 1;
        }
        Self(targets)
    }
    pub const fn find(&self, name: &str) -> Option<&Target> {
        let targets = &self.0;
        match find(targets, name) {
            Ok(i) => Some(&targets[i]),
            Err(_) => None,
        }
    }
}
impl Plan<[Target]> {
    pub const fn find<'a>(&'a self, name: &str) -> Option<&'a Target> {
        let targets = &self.0;
        match find(targets, name) {
            Ok(i) => Some(&targets[i]),
            Err(_) => None,
        }
    }
}
