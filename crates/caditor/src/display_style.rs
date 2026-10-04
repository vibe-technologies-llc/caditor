use crate::variants::all_variants;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DisplayStyle {
    #[default]
    ShadedWithEdges,
    Shaded,
    Wireframe,
}

all_variants!(DisplayStyle: ShadedWithEdges, Shaded, Wireframe);

impl DisplayStyle {
    pub fn id(self) -> &'static str {
        match self {
            Self::ShadedWithEdges => "view.style_shaded_with_edges",
            Self::Shaded => "view.style_shaded",
            Self::Wireframe => "view.style_wireframe",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::ShadedWithEdges => "Shaded with edges",
            Self::Shaded => "Shaded without edges",
            Self::Wireframe => "Wireframe",
        }
    }

    pub fn shows_faces(self) -> bool {
        !matches!(self, Self::Wireframe)
    }

    pub fn shows_edges(self) -> bool {
        !matches!(self, Self::Shaded)
    }
}
