//! At most one running query per entry, with one retry after configuration edits.
use std::collections::{HashMap, HashSet};
#[derive(Default)]
pub struct Requests {
    active: HashMap<String, u64>,
    pending: HashSet<String>,
}
impl Requests {
    pub fn begin(&mut self, id: &str, generation: u64) -> bool {
        if let Some(old) = self.active.get(id) {
            if *old != generation {
                self.pending.insert(id.into());
            }
            false
        } else {
            self.active.insert(id.into(), generation);
            true
        }
    }
    pub fn finish(&mut self, id: &str) -> bool {
        self.active.remove(id);
        self.pending.remove(id)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edits_queue_once_and_sources_are_independent() {
        let mut requests = Requests::default();
        assert!(requests.begin("a", 0));
        assert!(!requests.begin("a", 0));
        assert!(requests.begin("b", 0));
        assert!(!requests.begin("a", 1));
        assert!(!requests.begin("a", 2));
        assert!(!requests.finish("b"));
        assert!(requests.finish("a"));
        assert!(requests.begin("a", 2));
        assert!(!requests.finish("a"));
    }
}
