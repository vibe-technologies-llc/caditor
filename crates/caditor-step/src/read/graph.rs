use std::fmt;

use crate::part21::{Exchange, Instance, Parameter, Record};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Problem {
    pub entity: u64,
    pub reason: String,
}

impl Problem {
    pub fn new(entity: u64, reason: impl Into<String>) -> Self {
        Self {
            entity,
            reason: reason.into(),
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "#{} {}", self.entity, self.reason)
    }
}

pub(crate) type Read<T> = Result<T, Problem>;

#[derive(Clone, Copy)]
pub(crate) struct Graph<'a> {
    exchange: &'a Exchange,
}

#[derive(Clone, Copy)]
pub(crate) struct Entity<'a> {
    pub id: u64,
    pub instance: &'a Instance,
}

impl<'a> Graph<'a> {
    pub fn new(exchange: &'a Exchange) -> Self {
        Self { exchange }
    }

    pub fn entity(&self, id: u64) -> Read<Entity<'a>> {
        self.exchange
            .data
            .get(&id)
            .map(|instance| Entity { id, instance })
            .ok_or_else(|| Problem::new(id, "is missing from the file"))
    }

    pub fn entities(&self) -> impl Iterator<Item = Entity<'a>> + 'a {
        self.exchange
            .data
            .iter()
            .map(|(id, instance)| Entity { id: *id, instance })
    }
}

impl<'a> Entity<'a> {
    pub fn kind(&self) -> &'a str {
        match self.instance {
            Instance::Simple(record) => &record.name,
            Instance::Complex(records) => records
                .iter()
                .map(|record| record.name.as_str())
                .find(|name| {
                    !matches!(
                        *name,
                        "REPRESENTATION_ITEM" | "GEOMETRIC_REPRESENTATION_ITEM"
                    )
                })
                .unwrap_or_default(),
        }
    }

    pub fn is(&self, name: &str) -> bool {
        self.instance.record(name).is_some()
    }

    pub fn record(&self, name: &str) -> Read<Fields<'a>> {
        self.instance
            .record(name)
            .map(|record| Fields {
                id: self.id,
                record,
            })
            .ok_or_else(|| Problem::new(self.id, format!("is not a {}", friendly(name))))
    }

    pub fn fields(&self) -> Read<Fields<'a>> {
        match self.instance {
            Instance::Simple(record) => Ok(Fields {
                id: self.id,
                record,
            }),
            Instance::Complex(_) => Err(Problem::new(self.id, "combines several kinds")),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Fields<'a> {
    pub id: u64,
    record: &'a Record,
}

impl<'a> Fields<'a> {
    pub fn get(&self, index: usize) -> Read<&'a Parameter> {
        self.record
            .parameters
            .get(index)
            .ok_or_else(|| Problem::new(self.id, "has too few values"))
    }

    pub fn real(&self, index: usize) -> Read<f64> {
        self.get(index)?
            .real()
            .filter(|value| value.is_finite())
            .ok_or_else(|| Problem::new(self.id, "has a number that cannot be read"))
    }

    pub fn integer(&self, index: usize) -> Read<i64> {
        self.get(index)?
            .integer()
            .ok_or_else(|| Problem::new(self.id, "has a count that cannot be read"))
    }

    pub fn reference(&self, index: usize) -> Read<u64> {
        self.get(index)?
            .reference()
            .ok_or_else(|| Problem::new(self.id, "has a value where a reference belongs"))
    }

    pub fn optional_reference(&self, index: usize) -> Option<u64> {
        self.record.parameters.get(index)?.reference()
    }

    pub fn list(&self, index: usize) -> Read<&'a [Parameter]> {
        self.get(index)?
            .list()
            .ok_or_else(|| Problem::new(self.id, "has a value where a list belongs"))
    }

    pub fn logical(&self, index: usize) -> Read<bool> {
        self.get(index)?
            .logical()
            .ok_or_else(|| Problem::new(self.id, "has a value where true or false belongs"))
    }

    pub fn text(&self, index: usize) -> &'a str {
        self.record
            .parameters
            .get(index)
            .and_then(Parameter::text)
            .unwrap_or_default()
    }

    pub fn reals(&self, index: usize) -> Read<Vec<f64>> {
        self.list(index)?
            .iter()
            .map(|value| {
                value
                    .real()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| Problem::new(self.id, "has a number that cannot be read"))
            })
            .collect()
    }

    pub fn references(&self, index: usize) -> Read<Vec<u64>> {
        references(self.list(index)?, self.id)
    }
}

pub(crate) fn references(items: &[Parameter], context: u64) -> Read<Vec<u64>> {
    items
        .iter()
        .map(|item| {
            item.reference()
                .ok_or_else(|| Problem::new(context, "has a value where a reference belongs"))
        })
        .collect()
}

pub(crate) fn friendly(name: &str) -> String {
    name.to_ascii_lowercase().replace('_', " ")
}
