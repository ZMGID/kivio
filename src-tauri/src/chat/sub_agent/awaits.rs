use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

#[derive(Clone)]
struct Hold {
    root: String,
    owner: String,
    awaited: Vec<String>,
    since: u64,
    interruptible: bool,
}

#[derive(Default)]
pub(crate) struct AwaitGraph(Mutex<HashMap<String, Hold>>);

pub(crate) struct AwaitHold<'a> {
    graph: &'a AwaitGraph,
    id: String,
}

impl Drop for AwaitHold<'_> {
    fn drop(&mut self) {
        self.graph
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.id);
    }
}

impl AwaitGraph {
    pub fn hold(
        &self,
        root: &str,
        owner: &str,
        awaited: Vec<String>,
        since: u64,
        interruptible: bool,
    ) -> AwaitHold<'_> {
        let id = uuid::Uuid::new_v4().to_string();
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(
                id.clone(),
                Hold {
                    root: root.into(),
                    owner: owner.into(),
                    awaited,
                    since,
                    interruptible,
                },
            );
        AwaitHold { graph: self, id }
    }

    pub fn blocks_on(&self, root: &str, agent: &str, target: &str, question: u64) -> bool {
        let holds = self.0.lock().unwrap_or_else(|error| error.into_inner());
        fn blocked(
            holds: &HashMap<String, Hold>,
            root: &str,
            agent: &str,
            target: &str,
            question: u64,
            seen: &mut HashSet<String>,
        ) -> bool {
            if agent == target {
                return true;
            }
            if !seen.insert(agent.into()) {
                return false;
            }
            let blocked = holds
                .values()
                .filter(|hold| hold.root == root && hold.owner == agent)
                .filter(|hold| !hold.interruptible || hold.since >= question)
                .any(|hold| {
                    !hold.awaited.is_empty()
                        && hold
                            .awaited
                            .iter()
                            .all(|next| blocked(holds, root, next, target, question, seen))
                });
            seen.remove(agent);
            blocked
        }
        blocked(&holds, root, agent, target, question, &mut HashSet::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn any_of_escape_prevents_false_deadlock_and_holds_release() {
        let graph = AwaitGraph::default();
        let hold = graph.hold("r", "A", vec!["B".into(), "C".into()], 2, false);
        assert!(!graph.blocks_on("r", "A", "B", 1));
        let _c = graph.hold("r", "C", vec!["B".into()], 2, false);
        assert!(graph.blocks_on("r", "A", "B", 1));
        drop(hold);
        assert!(!graph.blocks_on("r", "A", "B", 1));
    }
    #[test]
    fn new_question_interrupts_an_older_interruptible_wait() {
        let graph = AwaitGraph::default();
        let _hold = graph.hold("r", "A", vec!["B".into()], 2, true);
        assert!(!graph.blocks_on("r", "A", "B", 3));
        assert!(graph.blocks_on("r", "A", "B", 1));
    }
}
