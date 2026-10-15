use std::fmt;

use crate::{
    part21::{Exchange, Instance, List, Parameter, Record},
    read::fault::Fault,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Problem {
    pub entity: u64,
    pub fault: Fault,
}

impl Problem {
    pub fn new(entity: u64, fault: Fault) -> Self {
        Self { entity, fault }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "#{} {}", self.entity, self.fault)
    }
}

pub(crate) type Read<T> = Result<T, Problem>;

#[derive(Clone, Copy)]
pub(crate) struct Graph<'a> {
    exchange: &'a Exchange<'a>,
}

#[derive(Clone, Copy)]
pub(crate) struct Entity<'a> {
    pub id: u64,
    pub instance: Instance<'a>,
}

impl<'a> Graph<'a> {
    pub fn new(exchange: &'a Exchange<'a>) -> Self {
        Self { exchange }
    }

    pub fn entity(&self, id: u64) -> Read<Entity<'a>> {
        self.exchange
            .instance(id)
            .map(|instance| Entity { id, instance })
            .ok_or_else(|| Problem::new(id, Fault::MissingFromFile))
    }

    pub fn entities(&self) -> impl Iterator<Item = Entity<'a>> + 'a {
        self.exchange
            .instances()
            .map(|(id, instance)| Entity { id, instance })
    }
}

impl<'a> Entity<'a> {
    pub fn kind(&self) -> &'a str {
        if let Some(record) = self.instance.simple() {
            return record.name();
        }
        self.instance
            .records()
            .map(Record::name)
            .find(|name| {
                !matches!(
                    *name,
                    "REPRESENTATION_ITEM" | "GEOMETRIC_REPRESENTATION_ITEM"
                )
            })
            .unwrap_or_default()
    }

    pub fn is(&self, name: &str) -> bool {
        self.instance.record(name).is_some()
    }

    pub fn record(&self, name: &str) -> Read<Fields<'a>> {
        self.find(name)
            .ok_or_else(|| Problem::new(self.id, Fault::NotOfKind(name.to_owned())))
    }

    pub fn find(&self, name: &str) -> Option<Fields<'a>> {
        self.instance.record(name).map(|record| Fields {
            id: self.id,
            record,
        })
    }

    pub fn fields(&self) -> Read<Fields<'a>> {
        self.instance
            .simple()
            .map(|record| Fields {
                id: self.id,
                record,
            })
            .ok_or_else(|| Problem::new(self.id, Fault::CombinesKinds))
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Fields<'a> {
    pub id: u64,
    record: Record<'a>,
}

impl<'a> Fields<'a> {
    pub fn get(&self, index: usize) -> Read<Parameter<'a>> {
        self.record
            .parameters()
            .get(index)
            .ok_or_else(|| Problem::new(self.id, Fault::TooFewValues))
    }

    pub fn real(&self, index: usize) -> Read<f64> {
        self.get(index)?
            .real()
            .filter(|value| value.is_finite())
            .ok_or_else(|| Problem::new(self.id, Fault::UnreadableNumber))
    }

    pub fn integer(&self, index: usize) -> Read<i64> {
        self.get(index)?
            .integer()
            .ok_or_else(|| Problem::new(self.id, Fault::UnreadableCount))
    }

    pub fn reference(&self, index: usize) -> Read<u64> {
        self.get(index)?
            .reference()
            .ok_or_else(|| Problem::new(self.id, Fault::ReferenceExpected))
    }

    pub fn optional_reference(&self, index: usize) -> Option<u64> {
        self.record.parameters().get(index)?.reference()
    }

    pub fn list(&self, index: usize) -> Read<List<'a>> {
        self.get(index)?
            .list()
            .ok_or_else(|| Problem::new(self.id, Fault::ListExpected))
    }

    pub fn logical(&self, index: usize) -> Read<bool> {
        self.get(index)?
            .logical()
            .ok_or_else(|| Problem::new(self.id, Fault::LogicalExpected))
    }

    pub fn name(&self, index: usize) -> Option<String> {
        let text = self.record.parameters().get(index)?.text()?;
        let trimmed = text.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    }

    pub fn reals(&self, index: usize) -> Read<Vec<f64>> {
        self.list(index)?
            .iter()
            .map(|value| {
                value
                    .real()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| Problem::new(self.id, Fault::UnreadableNumber))
            })
            .collect()
    }

    pub fn references(&self, index: usize) -> Read<Vec<u64>> {
        references(self.list(index)?, self.id)
    }
}

pub(crate) fn references(items: List<'_>, context: u64) -> Read<Vec<u64>> {
    items
        .iter()
        .map(|item| {
            item.reference()
                .ok_or_else(|| Problem::new(context, Fault::ReferenceExpected))
        })
        .collect()
}

pub(crate) fn friendly(name: &str) -> String {
    name.to_ascii_lowercase().replace('_', " ")
}
