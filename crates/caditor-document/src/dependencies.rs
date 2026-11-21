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
        self.cycle_from(target, &expression.parameters(), &Overlay::default())
    }

    pub fn cycle_overlaid(
        &self,
        overlay: &Overlay,
        target: ParameterId,
        first_steps: &BTreeSet<ParameterId>,
    ) -> Option<Vec<ParameterId>> {
        self.cycle_from(target, first_steps, overlay)
    }

    pub fn overlay_cycle(&self, overlay: &Overlay) -> Option<Vec<ParameterId>> {
        overlay
            .uses
            .iter()
            .find_map(|(id, used)| self.cycle_from(*id, used, overlay))
    }

    fn cycle_from(
        &self,
        target: ParameterId,
        first_steps: &BTreeSet<ParameterId>,
        overlay: &Overlay,
    ) -> Option<Vec<ParameterId>> {
        if first_steps.is_empty() {
            return None;
        }
        if first_steps.contains(&target) {
            return Some(vec![target, target]);
        }

        let mut toward_target = BTreeMap::new();
        let mut queue = VecDeque::from([target]);

        while let Some(current) = queue.pop_front() {
            for user in self.users_overlaid(current, overlay) {
                if user == target || toward_target.contains_key(&user) {
                    continue;
                }
                toward_target.insert(user, current);
                if first_steps.contains(&user) {
                    return Some(path_through(target, user, &toward_target));
                }
                queue.push_back(user);
            }
        }
        None
    }

    fn users_overlaid<'a>(
        &'a self,
        id: ParameterId,
        overlay: &'a Overlay,
    ) -> impl Iterator<Item = ParameterId> + 'a {
        let live = self
            .users
            .get(&id)
            .into_iter()
            .flatten()
            .filter(|user| !overlay.covers(**user))
            .copied();
        let held = overlay.users.get(&id).into_iter().flatten().copied();
        live.chain(held)
    }

    pub fn has_users(&self, id: ParameterId) -> bool {
        self.users.get(&id).is_some_and(|users| !users.is_empty())
    }

    pub fn forget(&mut self, id: ParameterId) {
        for old in self.uses.remove(&id).into_iter().flatten() {
            if let Some(users) = self.users.get_mut(&old) {
                users.remove(&id);
            }
        }
        self.users.remove(&id);
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

#[derive(Debug, Default)]
pub struct Overlay {
    uses: BTreeMap<ParameterId, BTreeSet<ParameterId>>,
    users: BTreeMap<ParameterId, BTreeSet<ParameterId>>,
}

impl Overlay {
    pub fn of<'a>(cells: impl IntoIterator<Item = (ParameterId, &'a Expression)>) -> Self {
        let mut overlay = Self::default();

        for (id, expression) in cells {
            let used = expression.parameters();

            for parameter in &used {
                overlay.users.entry(*parameter).or_default().insert(id);
            }
            overlay.uses.insert(id, used);
        }
        overlay
    }

    pub fn covers(&self, id: ParameterId) -> bool {
        self.uses.contains_key(&id)
    }

    pub fn is_empty(&self) -> bool {
        self.uses.is_empty()
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
