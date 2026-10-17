#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Service {
    Fusion,
}

impl Service {
    pub const ALL: [Self; 1] = [Self::Fusion];

    pub fn id(self) -> &'static str {
        match self {
            Self::Fusion => "fusion",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|service| service.id() == id)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Fusion => "Fusion 360",
        }
    }

    pub fn account_name(self) -> &'static str {
        match self {
            Self::Fusion => "Autodesk account",
        }
    }
}

impl std::fmt::Display for Service {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_service_is_found_again_by_its_id() {
        for service in Service::ALL {
            assert_eq!(Service::from_id(service.id()), Some(service));
        }
        assert_eq!(Service::from_id("onshape"), None);
    }
}
