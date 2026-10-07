use std::{collections::BTreeMap, ops::Deref};

use caditor_expression::{Expression, ParameterId};

use crate::document::Parameter;

#[derive(Debug, Clone, Default)]
pub struct ParameterList {
    items: Vec<Parameter>,
    positions: BTreeMap<ParameterId, usize>,
    named: BTreeMap<String, ParameterId>,
}

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
        self.positions.get(&id).copied()
    }

    pub fn get(&self, id: ParameterId) -> Option<&Parameter> {
        self.items.get(self.position(id)?)
    }

    pub fn named(&self, name: &str) -> Option<&Parameter> {
        self.get(*self.named.get(name)?)
    }

    pub fn insert(&mut self, index: usize, parameter: Parameter) {
        let index = index.min(self.items.len());
        self.positions.insert(parameter.id(), index);
        self.named.insert(parameter.name.clone(), parameter.id());
        self.items.insert(index, parameter);
        self.reindex_from(index + 1);
    }

    pub fn remove(&mut self, index: usize) -> Option<Parameter> {
        if index >= self.items.len() {
            return None;
        }
        let parameter = self.items.remove(index);
        self.positions.remove(&parameter.id());
        self.named.remove(&parameter.name);
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
        Some(std::mem::replace(&mut parameter.expression, expression))
    }

    pub fn set_note(&mut self, id: ParameterId, note: String) -> Option<String> {
        let index = self.position(id)?;
        let parameter = self.items.get_mut(index)?;
        Some(std::mem::replace(&mut parameter.note, note))
    }

    pub fn rename(&mut self, id: ParameterId, name: String) -> Option<String> {
        let index = self.position(id)?;
        let parameter = self.items.get_mut(index)?;
        let previous = std::mem::replace(&mut parameter.name, name.clone());
        self.named.remove(&previous);
        self.named.insert(name, id);
        Some(previous)
    }

    fn reindex_from(&mut self, start: usize) {
        for (index, parameter) in self.items.iter().enumerate().skip(start) {
            self.positions.insert(parameter.id(), index);
        }
    }
}

#[cfg(test)]
mod tests {
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
}
