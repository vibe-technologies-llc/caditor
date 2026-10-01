use std::collections::{BTreeMap, BTreeSet, VecDeque};

use caditor_expression::{Expression, ParameterId};

use crate::document::Document;

pub struct DependencyGraph {
    uses: BTreeMap<ParameterId, BTreeSet<ParameterId>>,
    users: BTreeMap<ParameterId, BTreeSet<ParameterId>>,
}

impl DependencyGraph {
    pub fn of(document: &Document) -> Self {
        let uses = document.parameter_dependencies();
        let mut users: BTreeMap<ParameterId, BTreeSet<ParameterId>> = BTreeMap::new();
        for (user, used) in &uses {
            for used in used {
                users.entry(*used).or_default().insert(*user);
            }
        }
        Self { uses, users }
    }

    pub fn cycle(&self, target: ParameterId, expression: &Expression) -> Option<Vec<ParameterId>> {
        let first_steps = expression.parameters();
        if first_steps.is_empty() {
            return None;
        }
        if first_steps.contains(&target) {
            return Some(vec![target, target]);
        }
        let mut toward_target = BTreeMap::new();
        let mut queue = VecDeque::from([target]);
        while let Some(current) = queue.pop_front() {
            for user in self.users.get(&current).into_iter().flatten() {
                if *user == target || toward_target.contains_key(user) {
                    continue;
                }
                toward_target.insert(*user, current);
                if first_steps.contains(user) {
                    return Some(path_through(target, *user, &toward_target));
                }
                queue.push_back(*user);
            }
        }
        None
    }

    pub fn set(&mut self, id: ParameterId, expression: &Expression) {
        let used = expression.parameters();
        for old in self.uses.get(&id).into_iter().flatten() {
            if let Some(users) = self.users.get_mut(old) {
                users.remove(&id);
            }
        }
        for new in &used {
            self.users.entry(*new).or_default().insert(id);
        }
        self.uses.insert(id, used);
    }
}

fn path_through(
    target: ParameterId,
    first_step: ParameterId,
    toward_target: &BTreeMap<ParameterId, ParameterId>,
) -> Vec<ParameterId> {
    let mut path = vec![target, first_step];
    let mut step = first_step;
    while let Some(next) = toward_target.get(&step).copied() {
        path.push(next);
        if next == target || path.len() > toward_target.len() + 2 {
            break;
        }
        step = next;
    }
    path
}
