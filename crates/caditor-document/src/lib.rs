use caditor_sketch::Sketch;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FeatureId(u64);

#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    pub name: String,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FeatureKind {
    Sketch(Sketch),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Feature {
    id: FeatureId,
    pub name: String,
    pub kind: FeatureKind,
}

impl Feature {
    pub fn id(&self) -> FeatureId {
        self.id
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Document {
    parameters: Vec<Parameter>,
    features: Vec<Feature>,
    next_feature_id: u64,
}

impl Document {
    pub fn parameters(&self) -> &[Parameter] {
        &self.parameters
    }

    pub fn features(&self) -> &[Feature] {
        &self.features
    }

    pub fn parameter(&self, name: &str) -> Option<f64> {
        self.parameters
            .iter()
            .find(|parameter| parameter.name == name)
            .map(|parameter| parameter.value)
    }

    pub fn set_parameter(&mut self, name: impl Into<String>, value: f64) {
        let name = name.into();
        match self
            .parameters
            .iter_mut()
            .find(|parameter| parameter.name == name)
        {
            Some(parameter) => parameter.value = value,
            None => self.parameters.push(Parameter { name, value }),
        }
    }

    pub fn add_feature(&mut self, name: impl Into<String>, kind: FeatureKind) -> FeatureId {
        let id = FeatureId(self.next_feature_id);
        self.next_feature_id += 1;
        self.features.push(Feature {
            id,
            name: name.into(),
            kind,
        });
        id
    }

    pub fn feature(&self, id: FeatureId) -> Option<&Feature> {
        self.features.iter().find(|feature| feature.id == id)
    }

    pub fn remove_feature(&mut self, id: FeatureId) -> Option<Feature> {
        let position = self.features.iter().position(|feature| feature.id == id)?;
        Some(self.features.remove(position))
    }
}

#[cfg(test)]
mod tests {
    use caditor_geometry::Plane;

    use super::*;

    fn sketch() -> FeatureKind {
        FeatureKind::Sketch(Sketch::new(Plane::XY))
    }

    #[test]
    fn set_parameter_inserts_then_overwrites() {
        let mut document = Document::default();
        document.set_parameter("width", 10.0);
        document.set_parameter("width", 12.5);

        assert_eq!(document.parameters().len(), 1);
        assert_eq!(document.parameter("width"), Some(12.5));
        assert_eq!(document.parameter("height"), None);
    }

    #[test]
    fn feature_ids_survive_removal_of_earlier_features() {
        let mut document = Document::default();
        let first = document.add_feature("First", sketch());
        let second = document.add_feature("Second", sketch());

        assert!(document.remove_feature(first).is_some());
        let third = document.add_feature("Third", sketch());

        assert_eq!(
            document
                .feature(second)
                .map(|feature| feature.name.as_str()),
            Some("Second")
        );
        assert_ne!(third, first);
        assert_eq!(document.feature(first), None);
    }
}
