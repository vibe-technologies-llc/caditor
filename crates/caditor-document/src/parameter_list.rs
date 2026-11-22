use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Deref,
    sync::{Arc, OnceLock},
};

use caditor_expression::{Expression, ParameterId};

use crate::{document::Parameter, model_parameters::ParameterOwner, tally};

#[derive(Clone, Debug, Default)]
pub struct ParameterList {
    items: Vec<Parameter>,
    index: Box<Index>,
}

#[derive(Clone, Debug, Default)]
struct Index {
    positions: BTreeMap<ParameterId, usize>,
    named: BTreeMap<String, ParameterId>,
    users: OnceLock<Arc<Users>>,
}

type Users = BTreeMap<ParameterId, BTreeSet<ParameterId>>;

impl PartialEq for ParameterList {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items
    }
}

impl Deref for ParameterList {
    type Target = [Parameter];

    fn deref(&self) -> &[Parameter] {
        &self.items
    }
}

impl ParameterList {
    pub fn position(&self, id: ParameterId) -> Option<usize> {
        self.index.positions.get(&id).copied()
    }

    pub fn get(&self, id: ParameterId) -> Option<&Parameter> {
        self.items.get(self.position(id)?)
    }

    pub fn named(&self, name: &str) -> Option<&Parameter> {
        self.get(*self.index.named.get(name)?)
    }

    pub fn users_of(&self, used: ParameterId) -> impl Iterator<Item = ParameterId> + '_ {
        self.index
            .users
            .get_or_init(|| Arc::new(users(&self.items)))
            .get(&used)
            .into_iter()
            .flatten()
            .copied()
    }

    pub fn insert(&mut self, index: usize, parameter: Parameter) {
        let index = index.min(self.items.len());

        self.enter(parameter.id(), parameter.expression.parameters());
        self.index.positions.insert(parameter.id(), index);
        self.index
            .named
            .insert(parameter.name.clone(), parameter.id());
        self.items.insert(index, parameter);
        self.reindex_from(index + 1);
    }

    pub fn remove(&mut self, index: usize) -> Option<Parameter> {
        if index >= self.items.len() {
            return None;
        }

        let parameter = self.items.remove(index);

        self.leave(parameter.id(), parameter.expression.parameters());
        self.index.positions.remove(&parameter.id());
        self.index.named.remove(&parameter.name);
        self.reindex_from(index);
        Some(parameter)
    }

    pub fn replace_expression(
        &mut self,
        id: ParameterId,
        expression: Expression,
    ) -> Option<Expression> {
        let index = self.position(id)?;
        let parameter = self.items.get_mut(index)?;
        let used = expression.parameters();
        let previous = std::mem::replace(&mut parameter.expression, expression);

        self.leave(id, previous.parameters());
        self.enter(id, used);
        Some(previous)
    }

    pub fn set_note(&mut self, id: ParameterId, note: String) -> Option<String> {
        let index = self.position(id)?;
        let parameter = self.items.get_mut(index)?;
        Some(std::mem::replace(&mut parameter.note, note))
    }

    pub fn set_owner(
        &mut self,
        id: ParameterId,
        owner: Option<ParameterOwner>,
    ) -> Option<Option<ParameterOwner>> {
        let index = self.position(id)?;
        let parameter = self.items.get_mut(index)?;
        Some(std::mem::replace(&mut parameter.owner, owner))
    }

    pub fn rename(&mut self, id: ParameterId, name: String) -> Option<String> {
        let index = self.position(id)?;
        let parameter = self.items.get_mut(index)?;
        let previous = std::mem::replace(&mut parameter.name, name.clone());

        self.index.named.remove(&previous);
        self.index.named.insert(name, id);
        Some(previous)
    }

    fn enter(&mut self, id: ParameterId, used: BTreeSet<ParameterId>) {
        if let Some(users) = self.index.users.get_mut() {
            let users = Arc::make_mut(users);
            for used in used {
                users.entry(used).or_default().insert(id);
            }
        }
    }

    fn leave(&mut self, id: ParameterId, used: BTreeSet<ParameterId>) {
        if let Some(users) = self.index.users.get_mut() {
            let users = Arc::make_mut(users);
            for used in used {
                if let Some(holders) = users.get_mut(&used) {
                    holders.remove(&id);
                    if holders.is_empty() {
                        users.remove(&used);
                    }
                }
            }
        }
    }

    fn reindex_from(&mut self, start: usize) {
        for (index, parameter) in self.items.iter().enumerate().skip(start) {
            self.index.positions.insert(parameter.id(), index);
        }
    }
}

fn users(items: &[Parameter]) -> Users {
    let mut users = Users::new();

    for parameter in items {
        tally::add(1);

        for used in parameter.expression.parameters() {
            users.entry(used).or_default().insert(parameter.id());
        }
    }
    users
}

#[cfg(test)]
mod tests {
    use caditor_expression::BinaryOperator;

    use super::*;

    fn parameter(raw: u64, name: &str) -> Parameter {
        Parameter::new(
            ParameterId::from_raw(raw),
            name.to_owned(),
            Expression::Number(raw as f64),
        )
    }

    fn id(raw: u64) -> ParameterId {
        ParameterId::from_raw(raw)
    }

    #[test]
    fn lookups_follow_inserts_in_the_middle_removals_and_renames() {
        let mut list = ParameterList::default();

        list.insert(0, parameter(1, "a"));
        list.insert(1, parameter(2, "b"));
        list.insert(1, parameter(3, "c"));

        let order: Vec<u64> = list.iter().map(|parameter| parameter.id().raw()).collect();
        let positions = [
            list.position(id(1)),
            list.position(id(3)),
            list.position(id(2)),
        ];

        assert_eq!(order, [1, 3, 2]);
        assert_eq!(positions, [Some(0), Some(1), Some(2)]);
        assert_eq!(list.named("c").map(Parameter::id), Some(id(3)));

        let removed = list.remove(0);

        assert_eq!(
            removed.map(|parameter| parameter.name),
            Some("a".to_owned())
        );
        assert_eq!(list.position(id(3)), Some(0));
        assert_eq!(list.position(id(2)), Some(1));
        assert!(list.get(id(1)).is_none());
        assert!(list.named("a").is_none());

        let previous = list.rename(id(2), "z".to_owned());

        assert_eq!(previous.as_deref(), Some("b"));
        assert!(list.named("b").is_none());
        assert_eq!(list.named("z").map(Parameter::id), Some(id(2)));
        assert_eq!(list.remove(5), None);
        assert_eq!(list.rename(id(9), "q".to_owned()), None);
    }

    #[test]
    fn equality_ignores_the_indexes() {
        let mut first = ParameterList::default();
        let mut second = ParameterList::default();

        first.insert(0, parameter(1, "a"));
        second.insert(0, parameter(1, "a"));

        assert_eq!(first, second);
        assert_eq!(
            second.replace_expression(id(1), Expression::Number(9.0)),
            Some(Expression::Number(1.0))
        );
        assert_ne!(first, second);
    }

    fn users_of(list: &ParameterList, used: u64) -> Vec<u64> {
        list.users_of(id(used)).map(ParameterId::raw).collect()
    }

    fn using(raw: u64, name: &str, used: &[u64]) -> Parameter {
        let expression = used
            .iter()
            .map(|other| Expression::Parameter(id(*other)))
            .reduce(|sum, next| Expression::binary(BinaryOperator::Add, sum, next))
            .unwrap_or(Expression::Number(0.0));
        Parameter::new(id(raw), name.to_owned(), expression)
    }

    #[test]
    fn the_users_of_a_parameter_follow_inserts_removals_and_replaced_expressions() {
        let mut list = ParameterList::default();

        list.insert(0, parameter(1, "a"));
        list.insert(1, using(2, "b", &[1]));

        let asked_first = users_of(&list, 1);

        list.insert(2, using(3, "c", &[1, 2]));

        let inserted = (users_of(&list, 1), users_of(&list, 2));

        list.replace_expression(id(3), Expression::Parameter(id(2)));

        let replaced = (users_of(&list, 1), users_of(&list, 2));

        list.replace_expression(id(2), Expression::Parameter(id(1)));

        let kept_shared = users_of(&list, 1);

        list.remove(1);

        let removed = (users_of(&list, 1), users_of(&list, 2));

        assert_eq!(asked_first, [2]);
        assert_eq!(inserted, (vec![2, 3], vec![3]));
        assert_eq!(replaced, (vec![2], vec![3]));
        assert_eq!(kept_shared, [2]);
        assert_eq!(removed, (vec![], vec![3]));
    }
}
